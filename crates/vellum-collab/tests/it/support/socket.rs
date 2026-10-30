// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A scripted editor over a real WebSocket, and an authorizer keyed by
//! its bearer token.

use std::time::Duration;

use async_trait::async_trait;
use axum::http::request::Parts;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use tokio_tungstenite::tungstenite::{Error as WsError, Message as WsMessage};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use vellum_collab::protocol::{self, MSG_DONE};
use vellum_collab::tree;
use vellum_collab::ws::Authorizer;
use vellum_collab::{Access, MarkdownCodec as _, RoomKey};
use vellum_kernel::{Error, ErrorKind, Result, UserId};
use yrs::sync::{Message, SyncMessage};
use yrs::{Doc, ReadTxn as _, Transact as _};

use super::TestCodec;

/// Grants write to `Bearer writer`, read to `Bearer reader`, nothing else.
pub struct HeaderAuthorizer;

#[async_trait]
impl Authorizer for HeaderAuthorizer {
    async fn authorize(
        &self,
        request: &Parts,
        _room: RoomKey,
    ) -> Result<(UserId, Access)> {
        let header = request
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());
        match header {
            Some("Bearer writer") => Ok((UserId::generate(), Access::Write)),
            Some("Bearer reader") => Ok((UserId::generate(), Access::Read)),
            Some(_) => {
                Err(Error::new(ErrorKind::PermissionDenied, "no access"))
            }
            None => Err(Error::new(ErrorKind::Unauthenticated, "sign in")),
        }
    }
}

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct WsEditor {
    pub socket: Socket,
    pub doc: Doc,
    pub dones: usize,
}

impl WsEditor {
    pub async fn connect(
        addr: &str,
        key: RoomKey,
        token: &str,
    ) -> std::result::Result<Self, WsError> {
        let url = format!(
            "ws://{addr}/collab/{}/{}?traceparent=00-4bf92f3577b34da6a3ce929d0e\
                0e4736-00f067aa0ba902b7-01",
            key.space, key.doc
        );
        let mut request = url.into_client_request()?;
        let _previous = request.headers_mut().insert(
            "authorization",
            format!("Bearer {token}").parse().unwrap(),
        );
        let (socket, _response) =
            tokio_tungstenite::connect_async(request).await?;
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

    pub async fn send(&mut self, messages: &[Message]) {
        self.socket
            .send(WsMessage::Binary(protocol::encode(messages).into()))
            .await
            .unwrap();
    }

    pub async fn edit(&mut self, change: impl FnOnce(&Doc)) {
        let before = self.doc.transact().state_vector();
        change(&self.doc);
        let update = self.doc.transact().encode_state_as_update_v1(&before);
        self.send(&[Message::Sync(SyncMessage::Update(update))])
            .await;
    }

    /// Reads frames until `until` holds; returns whether an update arrived.
    pub async fn read_until(
        &mut self,
        mut until: impl FnMut(&Self) -> bool,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !until(self) {
            let Ok(Some(Ok(message))) =
                tokio::time::timeout_at(deadline, self.socket.next()).await
            else {
                return false;
            };
            let WsMessage::Binary(frame) = message else {
                continue;
            };
            for message in protocol::decode(&frame).unwrap() {
                match message {
                    Message::Sync(SyncMessage::SyncStep1(sv)) => {
                        let update =
                            self.doc.transact().encode_state_as_update_v1(&sv);
                        self.send(&[Message::Sync(SyncMessage::SyncStep2(
                            update,
                        ))])
                        .await;
                    }
                    Message::Sync(
                        SyncMessage::SyncStep2(u) | SyncMessage::Update(u),
                    ) => {
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

    pub fn markdown(&self) -> String {
        TestCodec.to_markdown(&tree::read(&self.doc)).unwrap()
    }
}
