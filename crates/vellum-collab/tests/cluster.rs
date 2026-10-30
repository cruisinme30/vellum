// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Live editing across two replicas over real sockets (ADR 0037, D151 and
//! D278): an editor on either replica reaches the one copy of the room on
//! its owner, a forged forward is refused, and when the owner leaves the
//! other replica takes the room over from the shared log.
//!
//! The Postgres case uses the server at `VELLUM_TEST_POSTGRES`
//! (`postgres://user:pass@host:port`) when it is set, else a throwaway
//! container.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use support::socket::{HeaderAuthorizer, WsEditor};
use support::{TestCodec, add_paragraph, room};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt as _};
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use vellum_collab::cluster::{
    Cluster, ClusterConfig, FORWARD_HEADER, Membership, Node, Route,
};
use vellum_collab::memory::{MemoryFiles, MemoryUpdateStore};
use vellum_collab::sql::{Envelope, SqlUpdateStore};
use vellum_collab::ws;
use vellum_collab::{
    Access, CollabConfig, Hub, JoinRequest, RoomKey, UpdateStore,
};
use vellum_crypto::{KekId, LocalKeyring, SecretKey};
use vellum_db::sqlx::{self, Connection as _, PgConnection};
use vellum_db::{Index, IndexConfig};
use vellum_kernel::{ObjectId, Result, UserId};

const KEY: &[u8] = b"northstar-cluster-key-0123456789abcdef";

/// The replica list in memory.
#[derive(Default)]
struct Members(Mutex<BTreeMap<String, Node>>);

#[async_trait]
impl Membership for Members {
    async fn beat(&self, me: &Node) -> Result<()> {
        let _ = self.0.lock().unwrap().insert(me.name.clone(), me.clone());
        Ok(())
    }
    async fn live(&self, _window: Duration) -> Result<Vec<Node>> {
        Ok(self.0.lock().unwrap().values().cloned().collect())
    }
    async fn leave(&self, me: &Node) -> Result<()> {
        let _ = self.0.lock().unwrap().remove(&me.name);
        Ok(())
    }
}

/// One replica: a hub behind the clustered router on its own port.
struct Replica {
    hub: Hub,
    cluster: Arc<Cluster>,
    addr: String,
}

async fn replica(
    name: &str,
    hub: Hub,
    members: Arc<dyn Membership>,
) -> Replica {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let me = Node {
        name: name.to_owned(),
        address: addr.clone(),
    };
    let cluster = Arc::new(Cluster::new(
        ClusterConfig::new(me, KEY.to_vec()).unwrap(),
        members,
    ));
    let app = axum::Router::new().nest(
        "/collab",
        ws::clustered_router(
            hub.clone(),
            Arc::new(HeaderAuthorizer),
            Arc::clone(&cluster),
        ),
    );
    drop(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    Replica { hub, cluster, addr }
}

fn hub(store: Arc<dyn UpdateStore>, files: &Arc<MemoryFiles>) -> Hub {
    Hub::new(
        store,
        Arc::clone(files) as _,
        Arc::new(TestCodec),
        CollabConfig {
            batch_window: Duration::from_millis(10),
            ..CollabConfig::default()
        },
    )
}

/// Status of a handshake that carries `header` as its forwarded join.
async fn forwarded(addr: &str, key: RoomKey, header: &str) -> u16 {
    let url = format!("ws://{addr}/collab/{}/{}", key.space, key.doc);
    let mut request = url.into_client_request().unwrap();
    let _ = request
        .headers_mut()
        .insert(FORWARD_HEADER, header.parse().unwrap());
    match tokio_tungstenite::connect_async(request).await {
        Err(WsError::Http(response)) => response.status().as_u16(),
        Err(_) => 0,
        Ok(_) => 101,
    }
}

/// The whole story over two replicas that share `files` and one log.
async fn edit_across(a: &Replica, b: &Replica, files: &MemoryFiles) {
    // Each sees the other once both have beaten.
    for cluster in [&a.cluster, &b.cluster, &a.cluster] {
        cluster.tick().await.unwrap();
    }
    assert_eq!(a.cluster.live_nodes(), b.cluster.live_nodes());
    assert_eq!(a.cluster.live_nodes().len(), 2);

    // A room B owns, so the editor on A is forwarded.
    let key = std::iter::repeat_with(room)
        .find(|key| {
            matches!(
                a.cluster.route(key),
                Route::Remote(ref n) if n.address == b.addr
            )
        })
        .unwrap();
    assert_eq!(b.cluster.route(&key), Route::Local);
    files.put(key, "# Northstar launch\n");

    let mut alice = WsEditor::connect(&a.addr, key, "writer").await.unwrap();
    let mut bob = WsEditor::connect(&b.addr, key, "writer").await.unwrap();
    for editor in [&mut alice, &mut bob] {
        assert!(
            editor
                .read_until(|e| e.markdown() == "# Northstar launch\n")
                .await
        );
    }

    alice.edit(|doc| add_paragraph(doc, "Ship on Monday")).await;
    let one = "# Northstar launch\n\nShip on Monday\n";
    assert!(
        bob.read_until(|e| e.markdown() == one).await,
        "{}",
        bob.markdown()
    );
    bob.edit(|doc| add_paragraph(doc, "Docs by Friday")).await;
    let two = "# Northstar launch\n\nShip on Monday\n\nDocs by Friday\n";
    assert!(
        alice.read_until(|e| e.markdown() == two).await,
        "{}",
        alice.markdown()
    );

    // One copy of the room, on its owner.
    assert_eq!(a.hub.open_rooms(), 0);
    assert_eq!(b.hub.rooms(), vec![key]);

    // Only a join signed with the cluster key, for this room, is trusted.
    let join = JoinRequest {
        room: key,
        user: UserId::generate(),
        access: Access::Write,
        traceparent: None,
    };
    let stranger = Cluster::new(
        ClusterConfig::new(
            Node {
                name: "stranger".to_owned(),
                address: "127.0.0.1:9".to_owned(),
            },
            vec![7; 32],
        )
        .unwrap(),
        Arc::new(Members::default()),
    );
    assert_eq!(forwarded(&b.addr, key, &stranger.sign(&join)).await, 403);
    assert_eq!(forwarded(&b.addr, key, "v1.not.a.join").await, 403);
    let other_room = JoinRequest {
        room: room(),
        ..join
    };
    assert_eq!(
        forwarded(&b.addr, key, &b.cluster.sign(&other_room)).await,
        403
    );

    // B stops: it leaves the list and saves its room. Alice's relay ends,
    // and she reconnects to A, which now serves the room from the log.
    b.cluster.leave().await;
    b.hub.shutdown().await;
    a.cluster.tick().await.unwrap();
    assert_eq!(a.cluster.live_nodes().len(), 1);
    assert_eq!(a.cluster.route(&key), Route::Local);
    drop(alice);
    let mut again = WsEditor::connect(&a.addr, key, "writer").await.unwrap();
    assert!(
        again.read_until(|e| e.markdown() == two).await,
        "{}",
        again.markdown()
    );
    assert_eq!(a.hub.rooms(), vec![key]);
    a.hub.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_replicas_edit_one_room_over_sockets() {
    let members: Arc<dyn Membership> = Arc::new(Members::default());
    let store: Arc<dyn UpdateStore> = Arc::new(MemoryUpdateStore::new());
    let files = Arc::new(MemoryFiles::new());
    let a = replica(
        "pod-a",
        hub(Arc::clone(&store), &files),
        Arc::clone(&members),
    )
    .await;
    let b = replica("pod-b", hub(store, &files), members).await;
    edit_across(&a, &b, &files).await;
}

/// A fresh database on the test server, so the replica list holds only
/// this test's replicas.
async fn postgres() -> (Option<ContainerAsync<Postgres>>, String) {
    let (container, base) =
        if let Ok(base) = std::env::var("VELLUM_TEST_POSTGRES") {
            (None, base)
        } else {
            let container = Postgres::default()
                .with_tag("17-alpine")
                .start()
                .await
                .unwrap();
            let port = container.get_host_port_ipv4(5432).await.unwrap();
            (
                Some(container),
                format!("postgres://postgres:postgres@127.0.0.1:{port}"),
            )
        };
    let base = base.trim_end_matches('/').to_owned();
    let name =
        format!("collab_cluster_{}", ObjectId::generate().as_uuid().simple());
    let mut conn = PgConnection::connect(&format!("{base}/postgres"))
        .await
        .unwrap();
    let _ = sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&mut conn)
        .await
        .unwrap();
    (container, format!("{base}/{name}"))
}

/// The same key on every replica, as the data folder's key file gives.
fn keyring() -> LocalKeyring {
    LocalKeyring::new(
        KekId::new("test").unwrap(),
        SecretKey::from_bytes([9; 32]),
    )
}

/// The same story with the replica list and the change log in Postgres,
/// as `vellum serve` runs it: each replica has its own connection pool.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_replicas_share_a_postgres_cluster() {
    let (_container, url) = postgres().await;
    let files = Arc::new(MemoryFiles::new());
    let mut replicas = Vec::new();
    let mut indexes = Vec::new();
    for name in ["pod-a", "pod-b"] {
        let index = Index::open(&IndexConfig::postgres(&url)).await.unwrap();
        let store =
            Arc::new(SqlUpdateStore::new(index.clone(), Envelope(keyring())));
        let members = Arc::new(vellum_db::cluster::Nodes::new(index.clone()));
        replicas.push(replica(name, hub(store, &files), members).await);
        indexes.push(index);
    }
    edit_across(&replicas[0], &replicas[1], &files).await;

    // A replica that gave no address owns no rooms.
    let nodes = vellum_db::cluster::Nodes::new(indexes[0].clone());
    nodes
        .beat(&Node {
            name: "pod-c".to_owned(),
            address: String::new(),
        })
        .await
        .unwrap();
    let live = Membership::live(&nodes, Duration::from_secs(6))
        .await
        .unwrap();
    assert!(live.iter().all(|n| n.name != "pod-c"), "{live:?}");
    for index in indexes {
        index.close().await;
    }
}
