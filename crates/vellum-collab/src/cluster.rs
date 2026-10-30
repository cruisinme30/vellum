// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Rooms across several replicas: who owns a room, and how a connection
//! that lands elsewhere reaches it.
//!
//! Every replica beats in a shared heartbeat table ([`Membership`]) and
//! keeps the list of live replicas fresh. [`crate::placement::owner`] over
//! that list names the owner of each room, the same on every replica. An
//! editor whose WebSocket lands on another replica is authorized there and
//! then forwarded: that replica opens a WebSocket to the owner, carrying
//! the join it authorized in a signed header ([`Cluster::sign`]), and
//! relays frames both ways ([`crate::ws`]).
//!
//! When the list changes, rooms a replica no longer owns are released: their
//! updates are saved and their editors reconnect, which routes them to the
//! new owner. If the owner cannot be reached, the replica serves the room
//! itself, so a lost replica costs editors one reconnect. Two replicas that
//! briefly both hold a room still write one log, since the database
//! numbers updates (see [`crate::sql`]). See ADR 0037 (D151).

use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use vellum_kernel::{DocId, Error, ErrorKind, Result, SpaceId, UserId};

use crate::hub::{Access, Hub, JoinRequest, RoomKey};
use crate::placement;

/// The header that carries a forwarded join.
pub const FORWARD_HEADER: &str = "x-vellum-collab-forward";

/// How long a signed join may be used.
const FORWARD_TTL_MS: u64 = 30_000;

/// A replica: a stable name and the `host:port` other replicas reach.
pub use vellum_db::cluster::Node;

/// The shared list of live replicas.
#[async_trait]
pub trait Membership: Send + Sync + 'static {
    /// Records that `me` is alive.
    async fn beat(&self, me: &Node) -> Result<()>;
    /// The replicas that beat within `window`.
    async fn live(&self, window: Duration) -> Result<Vec<Node>>;
    /// Removes `me` on a clean shutdown.
    async fn leave(&self, me: &Node) -> Result<()>;
}

fn db_error(e: vellum_db::Error) -> Error {
    Error::new(ErrorKind::Unavailable, "the replica list is unavailable")
        .with_source(e)
}

#[async_trait]
impl Membership for vellum_db::cluster::Nodes {
    async fn beat(&self, me: &Node) -> Result<()> {
        Self::beat(self, me).await.map_err(db_error)
    }
    /// Replicas that gave no address cannot be reached, so they own no
    /// rooms (D278).
    async fn live(&self, window: Duration) -> Result<Vec<Node>> {
        let mut nodes = Self::live(self, window).await.map_err(db_error)?;
        nodes.retain(|node| !node.address.trim().is_empty());
        Ok(nodes)
    }
    async fn leave(&self, me: &Node) -> Result<()> {
        Self::leave(self, me).await.map_err(db_error)
    }
}

/// How a replica takes part in the cluster.
#[derive(Clone)]
pub struct ClusterConfig {
    /// This replica.
    pub me: Node,
    /// The key forwarded joins are signed with; the same on every replica,
    /// at least 32 bytes.
    pub key: Vec<u8>,
    /// Where the collab router is mounted (`/collab`).
    pub prefix: String,
    /// How often this replica beats and re-reads the list.
    pub beat_every: Duration,
    /// How long after its last beat a replica counts as gone.
    pub window: Duration,
}

impl std::fmt::Debug for ClusterConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterConfig")
            .field("me", &self.me)
            .field("prefix", &self.prefix)
            .field("beat_every", &self.beat_every)
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl ClusterConfig {
    /// A configuration with the default timings: a beat every 2 s, gone
    /// after 6 s of silence.
    ///
    /// # Errors
    ///
    /// `invalid_input` when the key is shorter than 32 bytes.
    pub fn new(me: Node, key: Vec<u8>) -> Result<Self> {
        if key.len() < 32 {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "the cluster key must be at least 32 bytes",
            ));
        }
        Ok(Self {
            me,
            key,
            prefix: "/collab".to_owned(),
            beat_every: Duration::from_secs(2),
            window: Duration::from_secs(6),
        })
    }
}

/// Where a connection for a room goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// This replica owns the room.
    Local,
    /// Another replica does.
    Remote(Node),
}

/// This replica's view of the cluster.
pub struct Cluster {
    config: ClusterConfig,
    membership: Arc<dyn Membership>,
    live: RwLock<Vec<Node>>,
}

impl std::fmt::Debug for Cluster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cluster")
            .field("config", &self.config)
            .field("live", &self.live_nodes())
            .finish_non_exhaustive()
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl Cluster {
    /// A cluster view over `membership`. Until the first [`Self::tick`]
    /// it knows only itself.
    #[must_use]
    pub fn new(config: ClusterConfig, membership: Arc<dyn Membership>) -> Self {
        let me = config.me.clone();
        Self {
            config,
            membership,
            live: RwLock::new(vec![me]),
        }
    }

    /// The configuration.
    #[must_use]
    pub const fn config(&self) -> &ClusterConfig {
        &self.config
    }

    /// The live replicas, this one always included.
    #[must_use]
    pub fn live_nodes(&self) -> Vec<Node> {
        self.live
            .read()
            .map_or_else(|_| vec![self.config.me.clone()], |l| l.clone())
    }

    /// Where a connection for `room` goes.
    #[must_use]
    pub fn route(&self, room: &RoomKey) -> Route {
        let nodes = self.live_nodes();
        let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();
        match placement::owner(room, &names) {
            Some(name) if *name != self.config.me.name => nodes
                .iter()
                .find(|n| n.name == *name)
                .map_or(Route::Local, |n| Route::Remote(n.clone())),
            _ => Route::Local,
        }
    }

    /// Beats and re-reads the list. On a database error the last list is
    /// kept, so a short outage moves no rooms.
    ///
    /// # Errors
    ///
    /// When the heartbeat table is unreachable.
    pub async fn tick(&self) -> Result<()> {
        self.membership.beat(&self.config.me).await?;
        let mut nodes = self.membership.live(self.config.window).await?;
        if !nodes.iter().any(|n| n.name == self.config.me.name) {
            nodes.push(self.config.me.clone());
        }
        nodes.sort();
        if let Ok(mut live) = self.live.write()
            && *live != nodes
        {
            info!(
                nodes = ?nodes.iter().map(|n| &n.name).collect::<Vec<_>>(),
                "collab replicas changed"
            );
            *live = nodes;
        }
        Ok(())
    }

    /// Releases every open room of `hub` this replica no longer owns, so its
    /// editors reconnect to the owner. Returns how many were released.
    pub async fn rebalance(&self, hub: &Hub) -> usize {
        let mut moved = 0;
        for room in hub.rooms() {
            if self.route(&room) != Route::Local {
                hub.release(room).await;
                moved += 1;
            }
        }
        if moved > 0 {
            info!(moved, "collab rooms handed to their new owners");
        }
        moved
    }

    /// Beats, refreshes and rebalances every [`ClusterConfig::beat_every`]
    /// until the task is aborted.
    #[must_use]
    pub fn spawn(self: Arc<Self>, hub: Hub) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut every = tokio::time::interval(self.config.beat_every);
            every.set_missed_tick_behavior(
                tokio::time::MissedTickBehavior::Delay,
            );
            loop {
                let _ = every.tick().await;
                match self.tick().await {
                    Ok(()) => {
                        let _ = self.rebalance(&hub).await;
                    }
                    Err(error) => {
                        warn!(%error, "collab replica list not refreshed")
                    }
                }
            }
        })
    }

    /// Leaves the list at once (on shutdown), so rooms move without
    /// waiting for the window.
    pub async fn leave(&self) {
        if let Err(error) = self.membership.leave(&self.config.me).await {
            warn!(%error, "could not leave the collab replica list");
        }
    }

    fn mac(&self, payload: &str) -> String {
        mac(&self.config.key, payload)
    }

    /// The header value that lets the owner trust a join authorized here.
    #[must_use]
    pub fn sign(&self, join: &JoinRequest) -> String {
        self.sign_at(join, now_ms())
    }

    fn sign_at(&self, join: &JoinRequest, now: u64) -> String {
        let access = if join.access.can_write() { "w" } else { "r" };
        let payload = format!(
            "v1.{}.{}.{}.{access}.{}",
            join.room.space,
            join.room.doc,
            join.user,
            now + FORWARD_TTL_MS
        );
        let mac = self.mac(&payload);
        format!("{payload}.{mac}")
    }

    /// The join a forwarded request carries, if its signature is good, it
    /// has not expired and it is for `room`.
    #[must_use]
    pub fn verify(
        &self,
        header: &str,
        room: RoomKey,
    ) -> Option<(UserId, Access)> {
        self.verify_at(header, room, now_ms())
    }

    fn verify_at(
        &self,
        header: &str,
        room: RoomKey,
        now: u64,
    ) -> Option<(UserId, Access)> {
        verify(&self.config.key, header, room, now)
    }
}

fn mac(key: &[u8], payload: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key)
        .unwrap_or_else(|_| unreachable!("HMAC takes any key length"));
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Whether `header` holds a valid forwarded join for the room `path` names.
///
/// `path` ends in `/{space_id}/{doc_id}`, with or without the mount
/// prefix; the join must be signed with `key` and unexpired. The gateway
/// uses it to exempt replica-to-replica forwards from
/// per-address rate limits (ADR 0040).
#[must_use]
pub fn is_signed_forward(key: &[u8], header: &str, path: &str) -> bool {
    let mut parts = path.trim_end_matches('/').rsplit('/');
    let (Some(doc), Some(space)) = (parts.next(), parts.next()) else {
        return false;
    };
    let (Ok(space), Ok(doc)) = (space.parse::<SpaceId>(), doc.parse::<DocId>())
    else {
        return false;
    };
    verify(key, header, RoomKey { space, doc }, now_ms()).is_some()
}

fn verify(
    key: &[u8],
    header: &str,
    room: RoomKey,
    now: u64,
) -> Option<(UserId, Access)> {
    let (payload, mac) = header.rsplit_once('.')?;
    let expected = self::mac(key, payload);
    if !bool::from(subtle::ConstantTimeEq::ct_eq(
        expected.as_bytes(),
        mac.as_bytes(),
    )) {
        return None;
    }
    let parts: Vec<&str> = payload.split('.').collect();
    let [version, space, doc, user, access, expires] = parts.as_slice() else {
        return None;
    };
    if *version != "v1" || expires.parse::<u64>().ok()? < now {
        return None;
    }
    if space.parse::<SpaceId>().ok()? != room.space
        || doc.parse::<DocId>().ok()? != room.doc
    {
        return None;
    }
    let access = match *access {
        "w" => Access::Write,
        "r" => Access::Read,
        _ => return None,
    };
    Some((user.parse().ok()?, access))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct Fake(Mutex<Vec<Node>>);

    #[async_trait]
    impl Membership for Fake {
        async fn beat(&self, me: &Node) -> Result<()> {
            let mut l = self
                .0
                .lock()
                .map_err(|_| Error::new(ErrorKind::Internal, "poisoned"))?;
            if !l.contains(me) {
                l.push(me.clone());
            }
            drop(l);
            Ok(())
        }
        async fn live(&self, _: Duration) -> Result<Vec<Node>> {
            Ok(self
                .0
                .lock()
                .map_err(|_| Error::new(ErrorKind::Internal, "poisoned"))?
                .clone())
        }
        async fn leave(&self, me: &Node) -> Result<()> {
            self.0
                .lock()
                .map_err(|_| Error::new(ErrorKind::Internal, "poisoned"))?
                .retain(|n| n != me);
            Ok(())
        }
    }

    fn node(name: &str) -> Node {
        Node {
            name: name.to_owned(),
            address: format!("{name}:7700"),
        }
    }

    fn cluster(name: &str, members: &Arc<Fake>) -> Cluster {
        let config = ClusterConfig::new(node(name), vec![7; 32])
            .unwrap_or_else(|e| unreachable!("{e}"));
        Cluster::new(config, Arc::clone(members) as Arc<dyn Membership>)
    }

    fn room(n: u128) -> RoomKey {
        RoomKey {
            space: SpaceId::from_u128(1),
            doc: DocId::from_u128(n),
        }
    }

    #[tokio::test]
    async fn every_replica_agrees_on_the_owner() -> Result<()> {
        let members = Arc::new(Fake::default());
        let (a, b, c) = (
            cluster("a", &members),
            cluster("b", &members),
            cluster("c", &members),
        );
        for x in [&a, &b, &c] {
            x.tick().await?;
        }
        for x in [&a, &b, &c] {
            x.tick().await?;
        }
        for n in 0..200 {
            let locals = [&a, &b, &c]
                .iter()
                .filter(|x| x.route(&room(n)) == Route::Local)
                .count();
            assert_eq!(locals, 1, "room {n} has {locals} owners");
        }
        Ok(())
    }

    #[tokio::test]
    async fn alone_or_before_the_first_tick_everything_is_local() {
        let members = Arc::new(Fake::default());
        let a = cluster("a", &members);
        assert!((0..50).all(|n| a.route(&room(n)) == Route::Local));
    }

    #[test]
    fn a_signed_join_is_accepted_once_and_only_for_its_room() {
        let members = Arc::new(Fake::default());
        let a = cluster("a", &members);
        let join = JoinRequest {
            room: room(1),
            user: UserId::from_u128(9),
            access: Access::Write,
            traceparent: None,
        };
        let header = a.sign_at(&join, 1_000);
        assert_eq!(
            a.verify_at(&header, room(1), 2_000),
            Some((join.user, Access::Write))
        );
        assert_eq!(a.verify_at(&header, room(2), 2_000), None);
        assert_eq!(
            a.verify_at(&header, room(1), 1_000 + FORWARD_TTL_MS + 1),
            None
        );
        let tampered = header.replacen(".w.", ".r.", 1);
        assert_eq!(a.verify_at(&tampered, room(1), 2_000), None);
        let other = Cluster::new(
            ClusterConfig::new(node("b"), vec![8; 32])
                .unwrap_or_else(|e| unreachable!("{e}")),
            members,
        );
        assert_eq!(other.verify_at(&header, room(1), 2_000), None);
    }

    #[test]
    fn a_signed_forward_is_recognised_from_its_path() {
        let members = Arc::new(Fake::default());
        let a = cluster("a", &members);
        let join = JoinRequest {
            room: room(1),
            user: UserId::from_u128(9),
            access: Access::Read,
            traceparent: None,
        };
        let header = a.sign(&join);
        let path = format!("/collab/{}/{}", room(1).space, room(1).doc);
        let key = &a.config.key;
        assert!(is_signed_forward(key, &header, &path));
        assert!(is_signed_forward(
            key,
            &header,
            path.trim_start_matches("/collab")
        ));
        let other = format!("/collab/{}/{}", room(2).space, room(2).doc);
        assert!(!is_signed_forward(key, &header, &other));
        let wrong: Vec<u8> = key.iter().map(|b| b ^ 0xff).collect();
        assert!(!is_signed_forward(&wrong, &header, &path));
        assert!(!is_signed_forward(key, "forged", &path));
        assert!(!is_signed_forward(key, &header, "/collab"));
    }
}
