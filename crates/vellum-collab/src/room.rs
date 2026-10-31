// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A room: one task that owns a document, its awareness and its peers.
//!
//! The task applies each incoming update, fans it out at once, and hands
//! batches to the room's writer ([`crate::persist`]) at most
//! [`CollabConfig::batch_window`] apart. Nothing in the loop waits on
//! storage except opening and closing.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::mem;
use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Duration, Instant, sleep_until};
use tracing::{Instrument as _, debug, error, info_span, warn};
use vellum_kernel::{Error, ErrorKind, Result, UserId};
use yrs::sync::awareness::AwarenessUpdate;
use yrs::sync::{Awareness, Message, SyncMessage};
use yrs::{ClientID, ReadTxn as _, StateVector, Transact as _};

use crate::config::CollabConfig;
use crate::hub::{
    Access, CloseReason, Deps, JoinRequest, Outbound, PeerId, Registry,
    RoomEntry, RoomKey,
};
use crate::metrics::label;
use crate::persist::{self, Job, Notify};
use crate::ports::{MarkdownVersion, VersionReason};
use crate::protocol::{self, MSG_DONE, MSG_PEERS, MSG_TRACE, PeerEntry};
use crate::trace::TraceParent;
use crate::tree;

/// A Yjs v1 update that changes nothing.
const EMPTY_UPDATE: [u8; 2] = [0, 0];

/// Commands queue behind each other; this bounds the queue per room.
const COMMAND_QUEUE: usize = 1024;

pub enum Command {
    Join {
        peer: PeerId,
        request: JoinRequest,
        tx: mpsc::Sender<Outbound>,
        reply: oneshot::Sender<JoinOutcome>,
    },
    Message {
        peer: PeerId,
        frame: Vec<u8>,
    },
    Done {
        reply: oneshot::Sender<Result<()>>,
    },
    ApplyMarkdown {
        markdown: String,
        reply: oneshot::Sender<Result<()>>,
    },
}

pub enum Control {
    Leave(PeerId),
    SetAccess {
        user: UserId,
        access: Option<Access>,
    },
    Shutdown(oneshot::Sender<()>),
}

pub enum JoinOutcome {
    Joined,
    /// The room closed before the join; open it again.
    Retry,
    Failed(Error),
}

struct Peer {
    user: UserId,
    access: Access,
    tx: mpsc::Sender<Outbound>,
    /// Awareness client IDs this connection owns.
    clients: HashSet<ClientID>,
    trace: Option<TraceParent>,
}

pub fn spawn(
    key: RoomKey,
    generation: u64,
    deps: Arc<Deps>,
    registry: Arc<Registry>,
) -> RoomEntry {
    let (cmd, cmd_rx) = mpsc::channel(COMMAND_QUEUE);
    let (ctl, ctl_rx) = mpsc::unbounded_channel();
    let room = Room {
        key,
        generation,
        writer: persist::spawn(key, Arc::clone(&deps)),
        deps,
        registry,
        awareness: Awareness::new(tree::new_doc()),
        peers: HashMap::new(),
        owners: HashMap::new(),
        pending: Vec::new(),
        pending_since: None,
        dirty: false,
        last_edit: Instant::now(),
        editors: BTreeSet::new(),
        last_markdown: None,
        empty_since: Some(Instant::now()),
        poisoned: false,
    };
    drop(tokio::spawn(room.run(cmd_rx, ctl_rx)));
    RoomEntry {
        generation,
        cmd,
        ctl,
    }
}

struct Room {
    key: RoomKey,
    generation: u64,
    deps: Arc<Deps>,
    registry: Arc<Registry>,
    writer: mpsc::UnboundedSender<Job>,
    awareness: Awareness,
    peers: HashMap<PeerId, Peer>,
    /// Which connection owns each awareness client ID.
    owners: HashMap<ClientID, PeerId>,
    /// Applied updates not yet handed to the writer.
    pending: Vec<Vec<u8>>,
    pending_since: Option<Instant>,
    /// Edited since the last Markdown version.
    dirty: bool,
    last_edit: Instant,
    editors: BTreeSet<UserId>,
    last_markdown: Option<String>,
    empty_since: Option<Instant>,
    /// The document may be inconsistent; the room must restart.
    poisoned: bool,
}

/// What the loop does after a timer fires.
enum Next {
    Continue,
    Close,
}

impl Room {
    fn config(&self) -> &CollabConfig {
        &self.deps.config
    }

    async fn run(
        mut self,
        mut cmd_rx: mpsc::Receiver<Command>,
        mut ctl_rx: mpsc::UnboundedReceiver<Control>,
    ) {
        let started = Instant::now();
        let span = info_span!("collab.load", room = %self.key);
        let loaded = self.load().instrument(span).await;
        self.deps.metrics.load_ms.record(
            started.elapsed().as_secs_f64() * 1000.0,
            &label("outcome", if loaded.is_ok() { "ok" } else { "error" }),
        );
        if let Err(err) = loaded {
            error!(room = %self.key, %err, "cannot open the room");
            self.registry.remove(self.key, self.generation);
            cmd_rx.close();
            while let Ok(command) = cmd_rx.try_recv() {
                reject(command, || {
                    JoinOutcome::Failed(Error::new(
                        err.kind(),
                        err.message().to_owned(),
                    ))
                });
            }
            return;
        }
        self.deps.metrics.rooms.add(1, &[]);

        let mut control_open = true;
        loop {
            let deadline = self.next_deadline();
            tokio::select! {
                biased;
                control = ctl_rx.recv(), if control_open => match control {
                    Some(control) => {
                        if let Some(ack) = self.on_control(control) {
                            self.close(
                                VersionReason::Shutdown,
                                &mut cmd_rx
                            ).await;
                            let _gone = ack.send(());
                            break;
                        }
                    }
                    None => control_open = false,
                },
                command = cmd_rx.recv() => match command {
                    Some(command) => {
                        self.on_command(command);
                        if self.poisoned {
                            self.abandon(&mut cmd_rx).await;
                            break;
                        }
                    }
                    None => break,
                },
                () = sleep_until(
                    deadline.unwrap_or_else(far_future)
                ), if deadline.is_some() => {
                    if matches!(
                        self.on_timer(),
                        Next::Close
                    ) && self.try_close(&mut cmd_rx).await {
                        break;
                    }
                }
            }
        }
        self.deps.metrics.rooms.add(-1, &[]);
    }

    async fn load(&mut self) -> Result<()> {
        let mut records = self.deps.store.load(self.key).await?;
        if records.is_empty()
            && let Some(markdown) =
                self.deps.files.read_markdown(self.key).await?
        {
            let seed = tree::new_doc();
            tree::write(&seed, &self.deps.codec.parse_markdown(&markdown)?);
            let update = seed
                .transact()
                .encode_state_as_update_v1(&StateVector::default());
            // Whether this node or a racing one seeded the log, the log
            // is now the source of truth.
            let _seeded = self.deps.store.seed(self.key, update).await?;
            records = self.deps.store.load(self.key).await?;
        }
        {
            let mut txn = self.awareness.doc().transact_mut();
            for record in records {
                let update =
                    protocol::decode_update(&record.update).map_err(|err| {
                        Error::new(
                            ErrorKind::Internal,
                            "the update log is corrupt",
                        )
                        .with_source(err)
                    })?;
                txn.apply_update(update).map_err(|err| {
                    Error::new(ErrorKind::Internal, "the update log is corrupt")
                        .with_source(err)
                })?;
            }
        }
        let current = tree::read(self.awareness.doc());
        self.last_markdown = self.deps.codec.to_markdown(&current).ok();
        Ok(())
    }

    fn next_deadline(&self) -> Option<Instant> {
        let config = self.config();
        [
            self.pending_since.map(|at| at + config.batch_window),
            self.dirty
                .then(|| self.last_edit + config.idle_version_after),
            self.empty_since.map(|at| at + config.close_after_empty),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn on_timer(&mut self) -> Next {
        let now = Instant::now();
        let config = self.config().clone();
        if self
            .pending_since
            .is_some_and(|at| now >= at + config.batch_window)
        {
            self.flush();
        }
        if self.dirty && now >= self.last_edit + config.idle_version_after {
            self.write_version(VersionReason::Idle, Notify::default());
        }
        if self
            .empty_since
            .is_some_and(|at| now >= at + config.close_after_empty)
        {
            return Next::Close;
        }
        Next::Continue
    }

    /// Closes the room if nobody is joining. Returns whether it closed.
    async fn try_close(
        &mut self,
        cmd_rx: &mut mpsc::Receiver<Command>,
    ) -> bool {
        self.flush();
        if self.dirty {
            self.write_version(VersionReason::Closed, Notify::default());
        }
        self.barrier().await;
        {
            let mut rooms = self.registry.lock();
            if !self.peers.is_empty() || !cmd_rx.is_empty() {
                drop(rooms);
                self.empty_since = self.peers.is_empty().then(Instant::now);
                return false;
            }
            if rooms
                .get(&self.key)
                .is_some_and(|e| e.generation == self.generation)
            {
                let _removed = rooms.remove(&self.key);
            }
        }
        drain_closed(cmd_rx);
        debug!(room = %self.key, "room closed");
        true
    }

    /// Shuts down: writes everything, then closes every connection.
    async fn close(
        &mut self,
        reason: VersionReason,
        cmd_rx: &mut mpsc::Receiver<Command>,
    ) {
        self.registry.remove(self.key, self.generation);
        cmd_rx.close();
        // Edits already queued are applied before the final version.
        while let Ok(command) = cmd_rx.try_recv() {
            match command {
                command @ Command::Message { .. } => self.on_command(command),
                other => reject(other, || JoinOutcome::Retry),
            }
        }
        self.flush();
        if self.dirty {
            self.write_version(reason, Notify::default());
        }
        self.barrier().await;
        for (_, peer) in self.peers.drain() {
            let _full_or_gone =
                peer.tx.try_send(Outbound::Close(CloseReason::Shutdown));
            self.deps.metrics.peers.add(-1, &[]);
        }
    }

    /// Gives up on a document that may be inconsistent: keeps the updates
    /// already accepted, writes no version, and sends every editor to a
    /// fresh room re-created from the log.
    async fn abandon(&mut self, cmd_rx: &mut mpsc::Receiver<Command>) {
        self.registry.remove(self.key, self.generation);
        drain_closed(cmd_rx);
        self.flush();
        self.barrier().await;
        for (_, peer) in self.peers.drain() {
            let _full_or_gone =
                peer.tx.try_send(Outbound::Close(CloseReason::Shutdown));
            self.deps.metrics.peers.add(-1, &[]);
        }
    }

    async fn barrier(&self) {
        let (done, wait) = oneshot::channel();
        if self.writer.send(Job::Barrier(done)).is_ok() {
            let _gone = wait.await;
        }
    }

    fn on_control(&mut self, control: Control) -> Option<oneshot::Sender<()>> {
        match control {
            Control::Leave(peer) => self.remove_peer(peer, None),
            Control::SetAccess { user, access } => {
                self.set_access(user, access);
            }
            Control::Shutdown(ack) => return Some(ack),
        }
        None
    }

    fn on_command(&mut self, command: Command) {
        match command {
            Command::Join {
                peer,
                request,
                tx,
                reply,
            } => {
                self.join(peer, &request, tx);
                let _gone = reply.send(JoinOutcome::Joined);
            }
            Command::Message { peer, frame } => self.on_frame(peer, &frame),
            Command::Done { reply } => {
                self.flush();
                self.write_version(
                    VersionReason::Done,
                    Notify {
                        peer: None,
                        reply: Some(reply),
                    },
                );
            }
            Command::ApplyMarkdown { markdown, reply } => {
                let _gone = reply.send(self.apply_markdown(&markdown));
            }
        }
    }

    fn join(
        &mut self,
        id: PeerId,
        request: &JoinRequest,
        tx: mpsc::Sender<Outbound>,
    ) {
        let span = info_span!(
            "collab.join",
            room = %self.key,
            peer = id.0,
            user = %request.user,
            access = ?request.access
        );
        if let Some(trace) = request.traceparent {
            trace.parent(&span);
        }
        let _entered = span.enter();
        let _previous = self.peers.insert(
            id,
            Peer {
                user: request.user,
                access: request.access,
                tx,
                clients: HashSet::new(),
                trace: request.traceparent,
            },
        );
        self.deps.metrics.peers.add(1, &[]);
        self.empty_since = None;

        let state_vector = self.awareness.doc().transact().state_vector();
        let mut greeting =
            vec![Message::Sync(SyncMessage::SyncStep1(state_vector))];
        if let Ok(states) = self.awareness.update()
            && !states.clients.is_empty()
        {
            greeting.push(Message::Awareness(states));
        }
        greeting.push(Message::Custom(
            MSG_PEERS,
            protocol::encode_peers(&self.peer_entries()),
        ));
        self.send_to(id, protocol::encode(&greeting).into());
    }

    fn on_frame(&mut self, id: PeerId, frame: &[u8]) {
        let Some(peer) = self.peers.get(&id) else {
            return;
        };
        if frame.len() > self.config().max_message_bytes {
            self.reject_message("too_large");
            self.remove_peer(id, Some(CloseReason::TooLarge));
            return;
        }
        let Ok(messages) = protocol::decode(frame) else {
            self.reject_message("malformed");
            self.remove_peer(id, Some(CloseReason::Protocol));
            return;
        };
        let span = info_span!(
            "collab.message",
            room = %self.key,
            peer = id.0,
            bytes = frame.len(),
            messages = messages.len()
        );
        if let Some(trace) = peer.trace {
            trace.parent(&span);
        }
        let started = Instant::now();
        let entered = span.enter();
        for message in messages {
            if !self.on_message(id, message) {
                self.reject_message("malformed");
                self.remove_peer(id, Some(CloseReason::Protocol));
                break;
            }
        }
        drop(entered);
        self.deps
            .metrics
            .apply_ms
            .record(started.elapsed().as_secs_f64() * 1000.0, &[]);
    }

    /// Handles one message; `false` drops the connection.
    fn on_message(&mut self, id: PeerId, message: Message) -> bool {
        let Some(access) = self.peers.get(&id).map(|peer| peer.access) else {
            return true;
        };
        match message {
            Message::Sync(SyncMessage::SyncStep1(state_vector)) => {
                let update = self
                    .awareness
                    .doc()
                    .transact()
                    .encode_state_as_update_v1(&state_vector);
                let reply = protocol::encode(&[Message::Sync(
                    SyncMessage::SyncStep2(update),
                )]);
                self.send_to(id, reply.into());
                true
            }
            Message::Sync(
                SyncMessage::SyncStep2(update) | SyncMessage::Update(update),
            ) => {
                if access.can_write() {
                    self.apply_update(Some(id), &update)
                } else {
                    self.reject_message("read_only");
                    true
                }
            }
            Message::Awareness(update) => {
                self.on_awareness(id, update);
                true
            }
            Message::AwarenessQuery => {
                if let Ok(states) = self.awareness.update() {
                    let reply = protocol::encode(&[Message::Awareness(states)]);
                    self.send_to(id, reply.into());
                }
                true
            }
            Message::Custom(MSG_TRACE, payload) => {
                let trace = std::str::from_utf8(&payload)
                    .ok()
                    .and_then(|text| text.parse().ok());
                if let Some(peer) = self.peers.get_mut(&id) {
                    peer.trace = trace;
                }
                true
            }
            Message::Custom(MSG_DONE, _) => {
                if access.can_write() {
                    self.flush();
                    let tx = self.peers.get(&id).map(|peer| peer.tx.clone());
                    self.write_version(
                        VersionReason::Done,
                        Notify {
                            peer: tx,
                            reply: None,
                        },
                    );
                } else {
                    self.reject_message("read_only");
                }
                true
            }
            // Auth is server-to-client; unknown custom tags are ignored so
            // newer editors can talk to older servers.
            Message::Auth(_) | Message::Custom(..) => {
                self.reject_message("unsupported");
                true
            }
        }
    }

    /// Applies an update from a peer (or from the server when `from` is
    /// `None`), fans it out and queues it for the log.
    fn apply_update(&mut self, from: Option<PeerId>, bytes: &[u8]) -> bool {
        let Ok(update) = protocol::decode_update(bytes) else {
            return false;
        };
        let doc = self.awareness.doc();
        let applied =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut txn = doc.transact_mut();
                txn.apply_update(update)
                    .is_ok()
                    .then(|| txn.encode_update_v1())
            }));
        let delta = match applied {
            Ok(Some(delta)) => delta,
            Ok(None) => return false,
            Err(_) => {
                // yrs panicked half-way through an update: the document
                // can no longer be trusted. Restart from the log.
                error!(room = %self.key, "applying an update panicked; \
                    restarting the room");
                self.poisoned = true;
                return false;
            }
        };
        if delta == EMPTY_UPDATE {
            return true;
        }
        self.record_edit(from, delta);
        true
    }

    fn record_edit(&mut self, from: Option<PeerId>, delta: Vec<u8>) {
        let now = Instant::now();
        self.deps.metrics.updates.add(1, &[]);
        self.deps
            .metrics
            .update_bytes
            .record(delta.len() as u64, &[]);
        if let Some(user) = from
            .and_then(|id| self.peers.get(&id))
            .map(|peer| peer.user)
        {
            let _new = self.editors.insert(user);
        }
        let frame: Bytes = protocol::encode(&[Message::Sync(
            SyncMessage::Update(delta.clone()),
        )])
        .into();
        self.pending.push(delta);
        let _ = self.pending_since.get_or_insert(now);
        self.dirty = true;
        self.last_edit = now;
        self.broadcast(&frame, from);
    }

    fn on_awareness(&mut self, id: PeerId, update: AwarenessUpdate) {
        let max_bytes = self.config().max_awareness_bytes;
        let max_clients = self.config().max_clients_per_peer;
        let Some(mut claimed) =
            self.peers.get(&id).map(|peer| peer.clients.len())
        else {
            return;
        };
        let mut accepted = HashMap::new();
        for (client, entry) in update.clients {
            if entry.json.len() > max_bytes {
                continue;
            }
            match self.owners.get(&client) {
                Some(holder) if *holder != id => continue,
                Some(_) => {}
                None => {
                    if claimed >= max_clients {
                        continue;
                    }
                    claimed += 1;
                }
            }
            let _previous = accepted.insert(client, entry);
        }
        if accepted.is_empty() {
            self.reject_message("awareness");
            return;
        }
        let filtered = AwarenessUpdate { clients: accepted };
        if self.awareness.apply_update(filtered.clone()).is_err() {
            return;
        }
        let mut membership_changed = false;
        for (client, entry) in &filtered.clients {
            let gone = &*entry.json == "null";
            if let Some(peer) = self.peers.get_mut(&id) {
                if gone {
                    membership_changed |= peer.clients.remove(client);
                    let _owner = self.owners.remove(client);
                } else if peer.clients.insert(*client) {
                    membership_changed = true;
                    let _owner = self.owners.insert(*client, id);
                }
            }
        }
        let frame: Bytes =
            protocol::encode(&[Message::Awareness(filtered)]).into();
        self.broadcast(&frame, Some(id));
        if membership_changed {
            self.broadcast_peers();
        }
    }

    fn set_access(&mut self, user: UserId, access: Option<Access>) {
        let affected: Vec<PeerId> = self
            .peers
            .iter()
            .filter(|(_, peer)| peer.user == user)
            .map(|(id, _)| *id)
            .collect();
        match access {
            None => {
                for id in affected {
                    self.remove_peer(id, Some(CloseReason::Revoked));
                }
            }
            Some(access) => {
                for id in &affected {
                    if let Some(peer) = self.peers.get_mut(id) {
                        peer.access = access;
                    }
                }
                if !affected.is_empty() {
                    self.broadcast_peers();
                }
            }
        }
    }

    /// Merges an external edit of the file. The file held the room's last
    /// version, so that version is the base of the three-way merge.
    fn apply_markdown(&mut self, markdown: &str) -> Result<()> {
        let theirs = self.deps.codec.parse_markdown(markdown)?;
        let base = match &self.last_markdown {
            Some(last) => self.deps.codec.parse_markdown(last)?,
            None => tree::read(self.awareness.doc()),
        };
        let delta = tree::merge(self.awareness.doc(), &base, &theirs);
        // The file now holds `theirs`; the next version is measured from it.
        self.last_markdown = self.deps.codec.to_markdown(&theirs).ok();
        if delta != EMPTY_UPDATE {
            // The file already holds this change: it needs a new version
            // only if the room had unsaved edits of its own.
            let had_edits = self.dirty;
            self.record_edit(None, delta);
            self.dirty = had_edits;
        }
        Ok(())
    }

    fn remove_peer(&mut self, id: PeerId, reason: Option<CloseReason>) {
        let Some(peer) = self.peers.remove(&id) else {
            return;
        };
        self.deps.metrics.peers.add(-1, &[]);
        if let Some(reason) = reason {
            let _full_or_gone = peer.tx.try_send(Outbound::Close(reason));
            self.deps
                .metrics
                .dropped_peers
                .add(1, &label("reason", reason.as_str()));
            warn!(
                room = %self.key,
                peer = id.0,
                reason = reason.as_str(),
                "dropped a connection"
            );
        }
        if !peer.clients.is_empty() {
            for client in &peer.clients {
                let _owner = self.owners.remove(client);
                self.awareness.remove_state(*client);
            }
            if let Ok(removed) = self
                .awareness
                .update_with_clients(peer.clients.iter().copied())
            {
                let frame: Bytes =
                    protocol::encode(&[Message::Awareness(removed)]).into();
                self.broadcast(&frame, None);
            }
            self.broadcast_peers();
        }
        if self.peers.is_empty() {
            self.empty_since = Some(Instant::now());
        }
    }

    fn peer_entries(&self) -> Vec<PeerEntry> {
        let mut entries: Vec<PeerEntry> = self
            .peers
            .values()
            .flat_map(|peer| {
                peer.clients.iter().map(|client| PeerEntry {
                    client_id: client.get(),
                    user: peer.user,
                    can_write: peer.access.can_write(),
                })
            })
            .collect();
        entries.sort_by_key(|entry| entry.client_id);
        entries
    }

    fn broadcast_peers(&mut self) {
        let payload = protocol::encode_peers(&self.peer_entries());
        let frame: Bytes =
            protocol::encode(&[Message::Custom(MSG_PEERS, payload)]).into();
        self.broadcast(&frame, None);
    }

    /// Sends to every peer but `except`; drops peers whose queue is full.
    fn broadcast(&mut self, frame: &Bytes, except: Option<PeerId>) {
        let mut slow = Vec::new();
        for (id, peer) in &self.peers {
            if Some(*id) == except {
                continue;
            }
            if peer.tx.try_send(Outbound::Frame(frame.clone())).is_err() {
                slow.push(*id);
            }
        }
        for id in slow {
            self.remove_peer(id, Some(CloseReason::SlowConsumer));
        }
    }

    fn send_to(&mut self, id: PeerId, frame: Bytes) {
        let delivered = self.peers.get(&id).is_some_and(|peer| {
            peer.tx.try_send(Outbound::Frame(frame)).is_ok()
        });
        if !delivered {
            self.remove_peer(id, Some(CloseReason::SlowConsumer));
        }
    }

    fn reject_message(&self, reason: &'static str) {
        self.deps.metrics.rejected.add(1, &label("reason", reason));
    }

    /// Hands pending updates to the writer as one batch.
    fn flush(&mut self) {
        self.pending_since = None;
        if self.pending.is_empty() {
            return;
        }
        let batch = mem::take(&mut self.pending);
        let merged = match batch.as_slice() {
            [single] => Ok(single.clone()),
            _ => yrs::merge_updates_v1(batch.iter().map(Vec::as_slice)),
        };
        match merged {
            Ok(update) => self.to_writer(Job::Append(update)),
            Err(err) => {
                warn!(room = %self.key, %err, "cannot merge a batch; writing \
                    it unmerged");
                for update in batch {
                    self.to_writer(Job::Append(update));
                }
            }
        }
    }

    fn write_version(&mut self, reason: VersionReason, notify: Notify) {
        let current = tree::read(self.awareness.doc());
        let markdown = match self.deps.codec.to_markdown(&current) {
            Ok(markdown) => markdown,
            Err(err) => {
                error!(room = %self.key, %err, "cannot serialize the document");
                // Try again after another quiet period, not in a loop.
                self.last_edit = Instant::now();
                if let Some(reply) = notify.reply {
                    let _gone = reply.send(Err(err));
                }
                return;
            }
        };
        self.dirty = false;
        let editors: Vec<UserId> =
            mem::take(&mut self.editors).into_iter().collect();
        if self.last_markdown.as_deref() == Some(markdown.as_str()) {
            // Nothing changed since the last version: acknowledge only.
            self.to_writer(Job::Ack(notify));
            return;
        }
        self.last_markdown = Some(markdown.clone());
        self.to_writer(Job::Version(
            MarkdownVersion {
                markdown,
                reason,
                editors,
            },
            notify,
        ));
    }

    fn to_writer(&self, job: Job) {
        if self.writer.send(job).is_err() {
            error!(room = %self.key, "the room writer stopped");
        }
    }
}

/// Answers a command that arrived after the room stopped taking work.
fn reject(command: Command, outcome: impl FnOnce() -> JoinOutcome) {
    match command {
        Command::Join { reply, .. } => {
            let _gone = reply.send(outcome());
        }
        Command::Done { reply } => {
            let _gone = reply.send(Ok(()));
        }
        Command::ApplyMarkdown { reply, .. } => {
            let _gone = reply.send(Err(Error::new(
                ErrorKind::Unavailable,
                "the collaboration room closed",
            )));
        }
        Command::Message { .. } => {}
    }
}

fn drain_closed(cmd_rx: &mut mpsc::Receiver<Command>) {
    cmd_rx.close();
    while let Ok(command) = cmd_rx.try_recv() {
        reject(command, || JoinOutcome::Retry);
    }
}

fn far_future() -> Instant {
    Instant::now() + Duration::from_hours(24)
}
