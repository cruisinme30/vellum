// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The WebSocket adapter: an axum router the gateway mounts raw
//! (`Gateway::mount("/collab", router)`), so its authentication, request
//! IDs and security headers apply.
//!
//! `GET /{space_id}/{doc_id}` upgrades to a WebSocket carrying the
//! [`crate::protocol`]. The [`Authorizer`] decides who the caller is and
//! what they may do before the upgrade; a caller it refuses never reaches
//! a room.
//!
//! With a [`Cluster`], a connection for a room another replica owns is
//! forwarded there after authorization (see [`crate::cluster`]), and a
//! request forwarded by a peer is trusted by its signed join instead of
//! being authorized again.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::extract::ws::{
    CloseFrame, Message as WsMessage, WebSocket, WebSocketUpgrade,
};
use axum::extract::{Path, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use futures_util::{SinkExt as _, StreamExt as _};
use tokio::time::{Instant, interval, interval_at};
use tracing::{Instrument as _, info_span, warn};
use vellum_kernel::{DocId, ErrorKind, Result, SpaceId, UserId};

use crate::cluster::{Cluster, FORWARD_HEADER, Node, Route};
use crate::hub::{Access, CloseReason, Hub, JoinRequest, Outbound, RoomKey};
use crate::trace::TraceParent;

/// How often the server pings an idle connection.
const PING_EVERY: Duration = Duration::from_secs(30);

/// Silence after which a connection is considered dead.
const DEAD_AFTER: Duration = Duration::from_secs(75);

/// Decides who is joining and what they may do. Deny by default: anything
/// but `Ok` refuses the upgrade.
#[async_trait]
pub trait Authorizer: Send + Sync + 'static {
    /// Authorizes the upgrade request for `room`. The gateway has already
    /// put the authenticated principal in the request extensions.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`], [`ErrorKind::PermissionDenied`] or
    /// [`ErrorKind::NotFound`] to refuse; any other kind is a failure.
    async fn authorize(
        &self,
        request: &Parts,
        room: RoomKey,
    ) -> Result<(UserId, Access)>;

    /// Checks an open connection again, with the upgrade request it was
    /// admitted on. The connection is closed as revoked unless this
    /// returns `Ok` for the same user; a changed [`Access`] applies at
    /// once. The default runs [`Self::authorize`] again.
    ///
    /// # Errors
    ///
    /// As [`Self::authorize`]; any error ends the connection.
    async fn recheck(
        &self,
        request: &Parts,
        room: RoomKey,
    ) -> Result<(UserId, Access)> {
        self.authorize(request, room).await
    }
}

/// How long a replica waits to reach a room's owner before serving the
/// room itself.
const FORWARD_CONNECT: Duration = Duration::from_secs(2);

#[derive(Clone)]
struct AppState {
    hub: Hub,
    authorizer: Arc<dyn Authorizer>,
    cluster: Option<Arc<Cluster>>,
}

/// The router to mount under the collab prefix, for a single replica.
pub fn router(hub: Hub, authorizer: Arc<dyn Authorizer>) -> Router {
    Router::new()
        .route("/{space_id}/{doc_id}", get(upgrade))
        .with_state(AppState {
            hub,
            authorizer,
            cluster: None,
        })
}

/// The router for one of several replicas: connections reach the replica
/// that owns their room. Start [`Cluster::spawn`] alongside it.
pub fn clustered_router(
    hub: Hub,
    authorizer: Arc<dyn Authorizer>,
    cluster: Arc<Cluster>,
) -> Router {
    Router::new()
        .route("/{space_id}/{doc_id}", get(upgrade))
        .with_state(AppState {
            hub,
            authorizer,
            cluster: Some(cluster),
        })
}

async fn upgrade(
    State(state): State<AppState>,
    Path((space, doc)): Path<(String, String)>,
    ws: WebSocketUpgrade,
    parts: Parts,
) -> Response {
    let (Ok(space), Ok(doc)) = (space.parse::<SpaceId>(), doc.parse::<DocId>())
    else {
        return (StatusCode::NOT_FOUND, "document not found").into_response();
    };
    let room = RoomKey { space, doc };
    // A join a peer forwarded: it authorized the editor already.
    let forwarded = parts
        .headers
        .get(FORWARD_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let (user, access, from_peer) = match (&state.cluster, forwarded) {
        (Some(cluster), Some(header)) => match cluster.verify(&header, room) {
            Some((user, access)) => (user, access, true),
            None => {
                return (StatusCode::FORBIDDEN, "bad forwarded join")
                    .into_response();
            }
        },
        (None, Some(_)) => {
            return (StatusCode::FORBIDDEN, "bad forwarded join")
                .into_response();
        }
        (_, None) => match state.authorizer.authorize(&parts, room).await {
            Ok((user, access)) => (user, access, false),
            Err(err) => {
                return (status_of(err.kind()), err.message().to_owned())
                    .into_response();
            }
        },
    };
    let request = JoinRequest {
        room,
        user,
        access,
        traceparent: traceparent(&parts.headers, &parts.uri),
    };
    let limit = state.hub.config().max_message_bytes;
    let hub = state.hub;
    // A forwarded join is served here even if this replica's list says
    // otherwise, so two lists that disagree for a moment never loop.
    let route = match &state.cluster {
        Some(cluster) if !from_peer => cluster.route(&room),
        _ => Route::Local,
    };
    let ws = ws.max_message_size(limit).max_frame_size(limit);
    let origin = parts.headers.get(axum::http::header::ORIGIN).cloned();
    // The replica the editor reached keeps checking their access; a
    // forwarded join was checked by that replica, which relays it.
    let recheck = (!from_peer).then(|| Recheck {
        authorizer: Arc::clone(&state.authorizer),
        request: parts,
        access,
        every: hub.config().recheck_access_every,
    });
    match (route, state.cluster) {
        (Route::Remote(owner), Some(cluster)) => {
            let hop = Hop {
                url: format!(
                    "ws://{}{}/{}/{}",
                    owner.address,
                    cluster.config().prefix,
                    room.space,
                    room.doc
                ),
                join: cluster.sign(&request),
                origin,
                owner,
            };
            ws.on_upgrade(move |socket| {
                forward(socket, hop, hub, request, recheck)
            })
        }
        _ => ws.on_upgrade(move |socket| {
            serve_checked(socket, hub, request, recheck)
        }),
    }
}

/// Where and how to forward one connection.
struct Hop {
    url: String,
    join: String,
    origin: Option<axum::http::HeaderValue>,
    owner: Node,
}

/// Relays an editor's socket to the room's owner. If the owner cannot be
/// reached in time (it may have just died), this replica serves the room.
async fn forward(
    socket: WebSocket,
    hop: Hop,
    hub: Hub,
    request: JoinRequest,
    recheck: Option<Recheck>,
) {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
    let span = info_span!(
        "collab.forward",
        room = %request.room,
        owner = %hop.owner.name
    );
    if let Some(trace) = request.traceparent {
        trace.parent(&span);
    }
    let upstream = async {
        let mut call = hop.url.as_str().into_client_request().ok()?;
        let headers = call.headers_mut();
        let _ = headers.insert(FORWARD_HEADER, hop.join.parse().ok()?);
        if let Some(origin) = &hop.origin {
            let _ = headers.insert(
                axum::http::header::ORIGIN,
                origin.to_str().ok()?.parse().ok()?,
            );
        }
        if let Some(trace) = request.traceparent {
            let _ =
                headers.insert("traceparent", trace.to_string().parse().ok()?);
        }
        match tokio::time::timeout(
            FORWARD_CONNECT,
            tokio_tungstenite::connect_async(call),
        )
        .await
        {
            Ok(Ok((stream, _))) => Some(stream),
            Ok(Err(err)) => {
                warn!(%err, "room owner unreachable; serving the room here");
                None
            }
            Err(_) => {
                warn!("room owner timed out; serving the room here");
                None
            }
        }
    }
    .instrument(span.clone())
    .await;
    match upstream {
        Some(stream) => {
            let (room, user) = (request.room, request.user);
            relay(
                socket,
                stream,
                Watch {
                    hub,
                    room,
                    user,
                    recheck,
                },
            )
            .instrument(span)
            .await;
        }
        None => serve_checked(socket, hub, request, recheck).await,
    }
}

/// Copies frames both ways until either side closes.
async fn relay<S>(
    socket: WebSocket,
    upstream: tokio_tungstenite::WebSocketStream<S>,
    mut watch: Watch,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio_tungstenite::tungstenite::Message as Up;
    let (mut client_tx, mut client_rx) = socket.split();
    let (mut up_tx, mut up_rx) = upstream.split();
    let mut check = watch.timer();
    loop {
        tokio::select! {
            _ = check.tick(), if watch.recheck.is_some() => {
                // The owner holds the room, so a changed access cannot be
                // applied from here: any change ends the relay, and the
                // editor reconnects with what they may do now.
                let before = watch.recheck.as_ref().map(|r| r.access);
                let kept = watch.run().await;
                if kept.is_none() || kept != before {
                    let _closed = up_tx.send(Up::Close(None)).await;
                    let reason = CloseReason::Revoked;
                    let _closed = client_tx.send(
                        close(reason.code(), reason.as_str())
                    ).await;
                    break;
                }
            }
            from_client = client_rx.next() => {
                let out = match from_client {
                    Some(Ok(WsMessage::Binary(b))) => Up::Binary(b),
                    Some(Ok(WsMessage::Text(t))) => Up::Text(t.as_str().into()),
                    Some(Ok(WsMessage::Ping(b))) => Up::Ping(b),
                    Some(Ok(WsMessage::Pong(b))) => Up::Pong(b),
                    Some(Ok(WsMessage::Close(_)) | Err(_)) | None => {
                        let _closed = up_tx.send(Up::Close(None)).await;
                        break;
                    }
                };
                if up_tx.send(out).await.is_err() {
                    let _closed = client_tx.send(
                        close(CloseReason::Shutdown.code(), "room closed")
                    ).await;
                    break;
                }
            }
            from_owner = up_rx.next() => {
                let out = match from_owner {
                    Some(Ok(Up::Binary(b))) => WsMessage::Binary(b),
                    Some(Ok(Up::Text(t))) => WsMessage::Text(t.as_str().into()),
                    Some(Ok(Up::Ping(b))) => WsMessage::Ping(b),
                    Some(Ok(Up::Pong(b))) => WsMessage::Pong(b),
                    Some(Ok(Up::Frame(_))) => continue,
                    Some(Ok(Up::Close(frame))) => {
                        let frame = frame.map(|f| CloseFrame {
                            code: f.code.into(),
                            reason: f.reason.as_str().into(),
                        });
                        let _closed = client_tx.send(
                            WsMessage::Close(frame)
                        ).await;
                        break;
                    }
                    Some(Err(_)) | None => {
                        let _closed = client_tx.send(
                            close(CloseReason::Shutdown.code(), "room closed")
                        ).await;
                        break;
                    }
                };
                if client_tx.send(out).await.is_err() {
                    let _closed = up_tx.send(Up::Close(None)).await;
                    break;
                }
            }
        }
    }
}

const fn status_of(kind: ErrorKind) -> StatusCode {
    match kind {
        ErrorKind::Unauthenticated => StatusCode::UNAUTHORIZED,
        ErrorKind::PermissionDenied => StatusCode::FORBIDDEN,
        ErrorKind::NotFound => StatusCode::NOT_FOUND,
        ErrorKind::InvalidInput => StatusCode::BAD_REQUEST,
        ErrorKind::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        ErrorKind::Timeout => StatusCode::GATEWAY_TIMEOUT,
        ErrorKind::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// The caller's trace context: the `traceparent` header, else the
/// `traceparent` query parameter (browsers cannot set upgrade headers).
#[must_use]
pub fn traceparent(headers: &HeaderMap, uri: &Uri) -> Option<TraceParent> {
    headers
        .get("traceparent")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .or_else(|| {
            uri.query()?
                .split('&')
                .find_map(|pair| pair.strip_prefix("traceparent="))
                .and_then(|value| value.parse().ok())
        })
}

/// What a connection needs to check its access again while it is open.
struct Recheck {
    authorizer: Arc<dyn Authorizer>,
    request: Parts,
    access: Access,
    every: Duration,
}

/// One connection's access re-checks.
struct Watch {
    hub: Hub,
    room: RoomKey,
    user: UserId,
    recheck: Option<Recheck>,
}

impl Watch {
    /// The re-check timer; it first fires one period from now.
    fn timer(&self) -> tokio::time::Interval {
        let every = self
            .recheck
            .as_ref()
            .map_or(Duration::from_hours(1), |r| r.every);
        interval_at(Instant::now() + every, every)
    }

    /// The access to keep, or `None` when the connection must end. A
    /// changed access is applied to the room when it is served here.
    async fn run(&mut self) -> Option<Access> {
        let recheck = self.recheck.as_mut()?;
        match recheck
            .authorizer
            .recheck(&recheck.request, self.room)
            .await
        {
            Ok((again, access)) if again == self.user => {
                if access != recheck.access {
                    recheck.access = access;
                    self.hub.set_access(self.room, self.user, Some(access));
                }
                Some(access)
            }
            Ok(_) => None,
            Err(err) if is_refusal(err.kind()) => None,
            // A failing check (the index is down, say) keeps the
            // connection as it is; the next tick tries again.
            Err(err) => {
                warn!(%err, "cannot check live-editing access again");
                Some(recheck.access)
            }
        }
    }
}

const fn is_refusal(kind: ErrorKind) -> bool {
    matches!(
        kind,
        ErrorKind::Unauthenticated
            | ErrorKind::PermissionDenied
            | ErrorKind::NotFound
    )
}

/// Bridges one upgraded socket to a room until either side ends.
///
/// The caller is trusted for the whole connection; the router's own
/// upgrades also re-check access every
/// [`CollabConfig::recheck_access_every`](crate::CollabConfig).
pub async fn serve(socket: WebSocket, hub: Hub, request: JoinRequest) {
    serve_checked(socket, hub, request, None).await;
}

async fn serve_checked(
    socket: WebSocket,
    hub: Hub,
    request: JoinRequest,
    recheck: Option<Recheck>,
) {
    let span = info_span!(
        "collab.connection",
        room = %request.room,
        user = %request.user
    );
    if let Some(trace) = request.traceparent {
        trace.parent(&span);
    }
    bridge(socket, hub, request, recheck).instrument(span).await;
}

async fn bridge(
    socket: WebSocket,
    hub: Hub,
    request: JoinRequest,
    recheck: Option<Recheck>,
) {
    let mut watch = Watch {
        hub: hub.clone(),
        room: request.room,
        user: request.user,
        recheck,
    };
    let mut check = watch.timer();
    let (mut sink, mut stream) = socket.split();
    let connection = match hub.join(request).await {
        Ok(connection) => connection,
        Err(err) => {
            warn!(%err, "cannot join the room");
            let _closed = sink.send(close(1011, "room unavailable")).await;
            return;
        }
    };
    let (sender, mut outbound) = connection.into_parts();
    let mut ping = interval(PING_EVERY);
    let mut last_heard = Instant::now();
    loop {
        tokio::select! {
            incoming = stream.next() => {
                last_heard = Instant::now();
                match incoming {
                    Some(Ok(WsMessage::Binary(frame))) => {
                        if sender.send(frame.to_vec()).await.is_err() {
                            let _closed = sink.send(
                                close(
                                    CloseReason::Shutdown.code(),
                                    "room closed"
                                )
                            ).await;
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Text(_))) => {
                        let _closed = sink.send(
                            close(1003, "binary frames only")
                        ).await;
                        break;
                    }
                    Some(Ok(WsMessage::Ping(_) | WsMessage::Pong(_))) => {}
                    Some(Ok(WsMessage::Close(_)) | Err(_)) | None => break,
                }
            }
            out = outbound.recv() => match out {
                Some(Outbound::Frame(frame)) => {
                    if sink.send(WsMessage::Binary(frame)).await.is_err() {
                        break;
                    }
                }
                Some(Outbound::Close(reason)) => {
                    let _closed = sink.send(
                        close(reason.code(), reason.as_str())
                    ).await;
                    break;
                }
                None => {
                    let _closed = sink.send(
                        close(CloseReason::Shutdown.code(), "room closed")
                    ).await;
                    break;
                }
            },
            _ = check.tick(), if watch.recheck.is_some() => {
                if watch.run().await.is_none() {
                    let reason = CloseReason::Revoked;
                    let _closed = sink.send(
                        close(reason.code(), reason.as_str())
                    ).await;
                    break;
                }
            }
            _ = ping.tick() => {
                if last_heard.elapsed() > DEAD_AFTER {
                    break;
                }
                if sink.send(
                    WsMessage::Ping(Vec::new().into())
                ).await.is_err() {
                    break;
                }
            }
        }
    }
}

fn close(code: u16, reason: &'static str) -> WsMessage {
    WsMessage::Close(Some(CloseFrame {
        code,
        reason: reason.into(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str =
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn reads_traceparent_from_header_then_query() {
        let mut headers = HeaderMap::new();
        let uri: Uri = format!("/s/d?x=1&traceparent={SAMPLE}")
            .parse()
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(
            traceparent(&headers, &uri)
                .map(|t| t.to_string())
                .as_deref(),
            Some(SAMPLE)
        );
        let other = "00-11111111111111111111111111111111-2222222222222222-00";
        let _previous = headers.insert(
            "traceparent",
            other.parse().unwrap_or_else(|e| unreachable!("{e}")),
        );
        assert_eq!(
            traceparent(&headers, &uri)
                .map(|t| t.to_string())
                .as_deref(),
            Some(other)
        );
        assert!(
            traceparent(&HeaderMap::new(), &Uri::from_static("/s/d")).is_none()
        );
    }
}
