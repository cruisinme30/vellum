// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! End to end over a real WebSocket: the axum router, authorization,
//! `traceparent`, two editors, Done, and keystroke latency on loopback.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::http::request::Parts;
use futures_util::StreamExt as _;
use support::socket::{HeaderAuthorizer, WsEditor};
use support::{add_paragraph, fixture, room, type_into};
use tokio_tungstenite::tungstenite::{Error as WsError, Message as WsMessage};
use vellum_collab::protocol::MSG_DONE;
use vellum_collab::ws::{self, Authorizer};
use vellum_collab::{Access, CollabConfig, RoomKey};
use vellum_kernel::{Error, ErrorKind, Result, UserId};
use yrs::sync::Message;

async fn serve(config: CollabConfig) -> (String, support::Fixture) {
    let f = fixture(config);
    let app = axum::Router::new().nest(
        "/collab",
        ws::router(f.hub.clone(), Arc::new(HeaderAuthorizer)),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    (addr, f)
}

fn status(result: std::result::Result<WsEditor, WsError>) -> u16 {
    match result {
        Err(WsError::Http(response)) => response.status().as_u16(),
        Err(_) => 0,
        Ok(_) => 101,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_browsers_edit_over_websockets() {
    let (addr, f) = serve(CollabConfig::default()).await;
    let key = room();
    f.files.put(key, "# Northstar launch\n");

    assert_eq!(status(WsEditor::connect(&addr, key, "nobody").await), 403);
    let missing = format!("ws://{addr}/collab/{}/{}", key.space, key.doc);
    let anonymous = tokio_tungstenite::connect_async(missing).await;
    assert!(matches!(anonymous, Err(WsError::Http(r)) if r.status() == 401));

    let mut alice = WsEditor::connect(&addr, key, "writer").await.unwrap();
    let mut bob = WsEditor::connect(&addr, key, "reader").await.unwrap();
    assert!(
        alice
            .read_until(|e| e.markdown() == "# Northstar launch\n")
            .await
    );
    assert!(
        bob.read_until(|e| e.markdown() == "# Northstar launch\n")
            .await
    );

    alice
        .edit(|doc| add_paragraph(doc, "Ship on 2026-11-24"))
        .await;
    let expected = "# Northstar launch\n\nShip on 2026-11-24\n";
    assert!(
        bob.read_until(|e| e.markdown() == expected).await,
        "{}",
        bob.markdown()
    );

    // The reader's edits are refused.
    bob.edit(|doc| add_paragraph(doc, "reader edit")).await;
    alice.send(&[Message::Custom(MSG_DONE, Vec::new())]).await;
    assert!(alice.read_until(|e| e.dones == 1).await);
    let versions = f.files.versions(key);
    assert_eq!(versions.last().map(|v| v.markdown.as_str()), Some(expected));

    // Keystroke to peer over loopback.
    let mut samples = Vec::new();
    for _ in 0..100 {
        let started = Instant::now();
        let before = bob.markdown();
        alice.edit(|doc| type_into(doc, 1, "!")).await;
        assert!(bob.read_until(|e| e.markdown() != before).await);
        samples.push(started.elapsed());
    }
    samples.sort();
    let p95 = samples[samples.len() * 95 / 100];
    println!("keystroke to peer over WebSocket: p95 {p95:?}");
    assert!(p95 < Duration::from_millis(50), "p95 {p95:?}");
}

/// One user whose access a test can change while they are connected.
struct Switch {
    user: UserId,
    state: std::sync::atomic::AtomicU8,
}

const WRITE: u8 = 0;
const READ: u8 = 1;

#[async_trait]
impl Authorizer for Switch {
    async fn authorize(
        &self,
        _request: &Parts,
        _room: RoomKey,
    ) -> Result<(UserId, Access)> {
        match self.state.load(std::sync::atomic::Ordering::SeqCst) {
            WRITE => Ok((self.user, Access::Write)),
            READ => Ok((self.user, Access::Read)),
            _ => Err(Error::new(ErrorKind::PermissionDenied, "access revoked")),
        }
    }
}

/// RT-028 (#459): access is checked again while a socket is open. Before, a
/// user who lost access kept reading and editing for as long as the
/// socket lived.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_editor_is_disconnected() {
    let f = fixture(CollabConfig {
        batch_window: Duration::from_millis(10),
        recheck_access_every: Duration::from_millis(100),
        ..CollabConfig::default()
    });
    let switch = Arc::new(Switch {
        user: UserId::generate(),
        state: std::sync::atomic::AtomicU8::new(WRITE),
    });
    let app = axum::Router::new().nest(
        "/collab",
        ws::router(f.hub.clone(), Arc::clone(&switch) as Arc<dyn Authorizer>),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    let key = room();
    f.files.put(key, "# Northstar\n");
    let mut editor = WsEditor::connect(&addr, key, "any").await.unwrap();
    assert!(editor.read_until(|e| e.markdown() == "# Northstar\n").await);

    // Revoked: the server closes the socket with 4403.
    switch.state.store(2, std::sync::atomic::Ordering::SeqCst);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut code = None;
    loop {
        let next =
            tokio::time::timeout_at(deadline, editor.socket.next()).await;
        match next {
            Ok(Some(Ok(WsMessage::Close(frame)))) => {
                code = frame.map(|f| u16::from(f.code));
                break;
            }
            Ok(Some(Ok(_))) => {}
            _ => break,
        }
    }
    assert_eq!(code, Some(4403));
}
