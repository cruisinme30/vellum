// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The stdio transport behind `vellum mcp`.
//!
//! One JSON-RPC message per line in each direction. Requests run
//! concurrently; `notifications/cancelled` stops one (including a
//! `subscriptions/listen` stream). When standard input closes, the server
//! ends open subscriptions, lets requests already received finish (up to
//! [`SHUTDOWN_GRACE`]), flushes their responses and exits. Credentials come from the environment (an API token), as the
//! specification asks for stdio, so no `OAuth` flow runs here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{
    AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader,
};
use tokio::sync::{mpsc, watch};
use tokio::task::{AbortHandle, JoinSet};

use crate::auth::Principal;
use crate::protocol::{Incoming, LEGACY_PROTOCOL_VERSION, Outgoing, RequestId};
use crate::server::{McpServer, Reply, RequestCtx};

/// Longest accepted line, in bytes.
const MAX_LINE: usize = 16 * 1024 * 1024;

/// How long requests already received may run after standard input closes.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

type InFlight = Arc<Mutex<HashMap<RequestId, AbortHandle>>>;

/// Serves MCP over a pair of byte streams until `input` ends.
///
/// # Errors
///
/// Returns an I/O error when reading or writing fails.
#[expect(
    clippy::too_many_lines,
    reason = "the read loop, cancellation and shutdown belong together"
)]
pub async fn serve<R, W>(
    server: Arc<McpServer>,
    principal: Principal,
    input: R,
    output: W,
) -> std::io::Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (tx, mut rx) = mpsc::channel::<String>(256);
    let writer = tokio::spawn(async move {
        let mut output = output;
        while let Some(line) = rx.recv().await {
            output.write_all(line.as_bytes()).await?;
            output.write_all(b"\n").await?;
            output.flush().await?;
        }
        Ok::<(), std::io::Error>(())
    });
    let in_flight: InFlight = Arc::default();
    let legacy_version: Arc<Mutex<Option<String>>> = Arc::default();
    let (shutdown, _) = watch::channel(false);
    let mut tasks = JoinSet::new();
    let mut reader = BufReader::new(input);
    let mut line = String::new();
    loop {
        line.clear();
        let n = (&mut reader)
            .take(u64::try_from(MAX_LINE).unwrap_or(u64::MAX))
            .read_line(&mut line)
            .await?;
        if n == 0 {
            break;
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let msg = match Incoming::parse(text) {
            Ok(m) => m,
            Err((id, error)) => {
                let _ = tx.send(Outgoing::Error { id, error }.to_line()).await;
                continue;
            }
        };
        if msg.id.is_none() && msg.method == "notifications/cancelled" {
            if let Some(id) = msg
                .params
                .get("requestId")
                .and_then(|v| serde_json::from_value::<RequestId>(v.clone()).ok())
                && let Some(handle) = in_flight
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id)
            {
                let reason = msg
                    .params
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                tracing::info!(request = %id, reason, "request cancelled by the client");
                handle.abort();
            }
            continue;
        }
        let fallback = legacy_version
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let ctx = RequestCtx {
            principal: principal.clone(),
            request_id: msg
                .id
                .as_ref()
                .map_or_else(|| "stdio".to_owned(), |id| format!("stdio-{id}")),
            fallback_version: fallback,
        };
        let id = msg.id.clone();
        let server = Arc::clone(&server);
        let tx = tx.clone();
        let flight = Arc::clone(&in_flight);
        let legacy = Arc::clone(&legacy_version);
        let task_id = id.clone();
        let mut closing = shutdown.subscribe();
        let handle = tasks.spawn(async move {
            let is_initialize = msg.method == "initialize";
            let reply = server.handle(msg, &ctx).await;
            if is_initialize && matches!(reply, Reply::Result { .. }) {
                // A legacy client: its later requests carry no _meta.
                *legacy.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some(LEGACY_PROTOCOL_VERSION.to_owned());
            }
            match reply {
                Reply::Listen(mut listener) => {
                    let _ = tx.send(listener.acknowledgement().to_line()).await;
                    loop {
                        let next = tokio::select! {
                            n = listener.next() => n,
                            // Input closed: nobody is left to listen.
                            _ = closing.wait_for(|closed| *closed) => break,
                        };
                        if let Some(n) = next {
                            if tx.send(n.to_line()).await.is_err() {
                                break;
                            }
                        } else {
                            let _ = tx.send(listener.closing().to_line()).await;
                            break;
                        }
                    }
                }
                other => {
                    if let Some(out) = other.to_outgoing() {
                        let _ = tx.send(out.to_line()).await;
                    }
                }
            }
            if let Some(id) = &task_id {
                let _ = flight
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(id);
            }
        });
        if let Some(id) = id {
            let mut map = in_flight.lock().unwrap_or_else(PoisonError::into_inner);
            map.retain(|_, h| !h.is_finished());
            if !handle.is_finished() {
                let _ = map.insert(id, handle);
            }
        }
        // Reap finished tasks so the set does not grow without bound.
        while tasks.try_join_next().is_some() {}
    }
    // Standard input closed. End subscriptions, but answer the requests
    // already received: a client may write its last request and close
    // its end straight away.
    let _ = shutdown.send(true);
    let drained = tokio::time::timeout(SHUTDOWN_GRACE, async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    if drained.is_err() {
        tracing::warn!("requests still running after standard input closed; stopping them");
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
    drop(tx);
    writer.await.map_err(std::io::Error::other)?
}
