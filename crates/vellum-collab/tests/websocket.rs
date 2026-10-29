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
use futures_util::{SinkExt as _, StreamExt as _};
use support::{TestCodec, add_paragraph, fixture, room, type_into};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::{Error as WsError, Message as WsMessage};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use vellum_collab::protocol::{self, MSG_DONE};
use vellum_collab::tree;
use vellum_collab::ws::{self, Authorizer};
use vellum_collab::{Access, CollabConfig, MarkdownCodec as _, RoomKey};
use vellum_kernel::{Error, ErrorKind, Result, UserId};
use yrs::sync::{Message, SyncMessage};
use yrs::{Doc, ReadTxn as _, Transact as _};

/// Grants write to `Bearer writer`, read to `Bearer reader`, nothing else.
struct HeaderAuthorizer;

#[async_trait]
impl Authorizer for HeaderAuthorizer {
    async fn authorize(&self, request: &Parts, _room: RoomKey) -> Result<(UserId, Access)> {
        let header = request
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        match header {
            Some("Bearer writer") => Ok((UserId::generate(), Access::Write)),
            Some("Bearer reader") => Ok((UserId::generate(), Access::Read)),
            Some(_) => Err(Error::new(ErrorKind::PermissionDenied, "no access")),
            None => Err(Error::new(ErrorKind::Unauthenticated, "sign in")),
        }
    }
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct WsEditor {
    socket: Socket,
    doc: Doc,
    dones: usize,
}

impl WsEditor {
    async fn connect(addr: &str, key: RoomKey, token: &str) -> std::result::Result<Self, WsError> {
        let url = format!(
            "ws://{addr}/collab/{}/{}?traceparent=00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            key.space, key.doc
        );
        let mut request = url.into_client_request()?;
        let _previous = request
            .headers_mut()
            .insert("authorization", format!("Bearer {token}").parse().unwrap());
        let (socket, _response) = tokio_tungstenite::connect_async(request).await?;
        let mut editor = Self {
            socket,
            doc: tree::new_doc(),
            dones: 0,
        };
        let state_vector = editor.doc.transact().state_vector();
        editor
            .send(&[Message::Sync(SyncMessage::SyncStep1(state_vector))])
            .await;
        Ok(editor)
    }

    async fn send(&mut self, messages: &[Message]) {
        self.socket
            .send(WsMessage::Binary(protocol::encode(messages).into()))
            .await
            .unwrap();
    }

    async fn edit(&mut self, change: impl FnOnce(&Doc)) {
        let before = self.doc.transact().state_vector();
        change(&self.doc);
        let update = self.doc.transact().encode_state_as_update_v1(&before);
        self.send(&[Message::Sync(SyncMessage::Update(update))])
            .await;
    }

    /// Reads frames until `until` holds; returns whether an update arrived.
    async fn read_until(&mut self, mut until: impl FnMut(&Self) -> bool) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !until(self) {
            let Ok(Some(Ok(message))) = tokio::time::timeout_at(deadline, self.socket.next()).await
            else {
                return false;
            };
            let WsMessage::Binary(frame) = message else {
                continue;
            };
            for message in protocol::decode(&frame).unwrap() {
                match message {
                    Message::Sync(SyncMessage::SyncStep1(sv)) => {
                        let update = self.doc.transact().encode_state_as_update_v1(&sv);
                        self.send(&[Message::Sync(SyncMessage::SyncStep2(update))])
                            .await;
                    }
                    Message::Sync(SyncMessage::SyncStep2(u) | SyncMessage::Update(u)) => {
                        let update = protocol::decode_update(&u).unwrap();
                        self.doc.transact_mut().apply_update(update).unwrap();
                    }
                    Message::Custom(MSG_DONE, _) => self.dones += 1,
                    _ => {}
                }
            }
        }
        true
    }

    fn markdown(&self) -> String {
        TestCodec.to_markdown(&tree::read(&self.doc)).unwrap()
    }
}

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
