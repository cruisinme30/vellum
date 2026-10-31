// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! One writer task per room, so storage latency never delays fan-out.
//!
//! Jobs run in order. A batch that cannot be appended after
//! [`crate::CollabConfig::persist_attempts`] tries is kept and merged into
//! the next one, so a storage outage delays writes but never drops them.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tracing::{Instrument as _, error, info_span, warn};
use vellum_kernel::{Error, ErrorKind, Result};

use crate::hub::{Deps, Outbound, RoomKey};
use crate::metrics::label;
use crate::ports::{MarkdownVersion, Seq};
use crate::protocol::{self, MSG_DONE};

/// Who to tell once a version is written.
#[derive(Debug, Default)]
pub struct Notify {
    /// The editor who pressed Done gets a [`MSG_DONE`] frame.
    pub peer: Option<mpsc::Sender<Outbound>>,
    /// An API caller waiting on [`crate::Hub::done`].
    pub reply: Option<oneshot::Sender<Result<()>>>,
}

impl Notify {
    fn finish(self, result: Result<()>) {
        if result.is_ok()
            && let Some(peer) = self.peer
        {
            let frame = protocol::encode(&[yrs::sync::Message::Custom(
                MSG_DONE,
                Vec::new(),
            )]);
            let _full_or_gone = peer.try_send(Outbound::Frame(frame.into()));
        }
        if let Some(reply) = self.reply {
            let _gone = reply.send(result);
        }
    }
}

pub enum Job {
    /// Append one merged batch to the log.
    Append(Vec<u8>),
    /// Write a Markdown version, then compact the log.
    Version(MarkdownVersion, Notify),
    /// Acknowledge a Done that found nothing new to write.
    Ack(Notify),
    /// Reply once every earlier job is done.
    Barrier(oneshot::Sender<()>),
}

pub fn spawn(room: RoomKey, deps: Arc<Deps>) -> mpsc::UnboundedSender<Job> {
    let (tx, rx) = mpsc::unbounded_channel();
    let writer = Writer {
        room,
        deps,
        last_seq: None,
        backlog: Vec::new(),
    };
    drop(tokio::spawn(writer.run(rx)));
    tx
}

struct Writer {
    room: RoomKey,
    deps: Arc<Deps>,
    /// The last record this room appended; compaction stops here.
    last_seq: Option<Seq>,
    /// Updates not yet appended.
    backlog: Vec<Vec<u8>>,
}

impl Writer {
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Job>) {
        while let Some(job) = rx.recv().await {
            match job {
                Job::Append(update) => {
                    self.backlog.push(update);
                    self.append().await;
                }
                Job::Version(version, notify) => {
                    self.append().await;
                    let result = self.write_version(version).await;
                    let written = result.is_ok();
                    notify.finish(result);
                    if written {
                        self.compact().await;
                    }
                }
                Job::Ack(notify) => {
                    self.append().await;
                    notify.finish(Ok(()));
                }
                Job::Barrier(done) => {
                    self.append().await;
                    let _gone = done.send(());
                }
            }
        }
        if !self.backlog.is_empty() {
            error!(
                room = %self.room,
                updates = self.backlog.len(),
                "room closed with unwritten updates; editors still hold them \
                    and resend on reconnect"
            );
        }
    }

    async fn append(&mut self) {
        if self.backlog.is_empty() {
            return;
        }
        let merged = match merge(&self.backlog) {
            Ok(merged) => merged,
            Err(err) => {
                error!(room = %self.room, %err, "cannot merge updates");
                return;
            }
        };
        let span = info_span!(
            "collab.persist",
            room = %self.room,
            bytes = merged.len(),
            updates = self.backlog.len(),
            seq = tracing::field::Empty
        );
        let deps = Arc::clone(&self.deps);
        let room = self.room;
        let result =
            retry(&deps, "append", || deps.store.append(room, merged.clone()))
                .instrument(span.clone())
                .await;
        match result {
            Ok(seq) => {
                let _span = span.record("seq", seq);
                self.last_seq = Some(seq);
                self.backlog.clear();
            }
            Err(err) => {
                error!(room = %self.room, %err, "update log unavailable; \
                    keeping the batch for the next write");
                self.backlog = vec![merged];
            }
        }
    }

    async fn write_version(&self, version: MarkdownVersion) -> Result<()> {
        let reason = version.reason.as_str();
        let span = info_span!(
            "collab.version",
            room = %self.room,
            reason,
            bytes = version.markdown.len()
        );
        let deps = Arc::clone(&self.deps);
        let room = self.room;
        let result = retry(&deps, "version", || {
            deps.files.write_version(room, version.clone())
        })
        .instrument(span)
        .await;
        match &result {
            Ok(()) => {
                self.deps.metrics.versions.add(1, &label("reason", reason));
            }
            Err(err) => {
                error!(room = %self.room, %err, "cannot write the Markdown \
                    version");
            }
        }
        result
    }

    /// Folds every record up to the last one this room wrote into one, so
    /// the next load replays a snapshot plus a short tail.
    async fn compact(&self) {
        let Some(through) = self.last_seq else {
            return;
        };
        let span = info_span!("collab.compact", room = %self.room, through);
        let result: Result<()> = async {
            let records = self.deps.store.load(self.room).await?;
            let prefix: Vec<Vec<u8>> = records
                .into_iter()
                .filter(|record| record.seq <= through)
                .map(|record| record.update)
                .collect();
            if prefix.len() < 2 {
                return Ok(());
            }
            let merged = merge(&prefix)?;
            self.deps.store.compact(self.room, through, merged).await
        }
        .instrument(span)
        .await;
        if let Err(err) = result {
            warn!(room = %self.room, %err, "log compaction failed; the log \
                stays longer");
        }
    }
}

fn merge(updates: &[Vec<u8>]) -> Result<Vec<u8>> {
    match updates {
        [single] => Ok(single.clone()),
        _ => yrs::merge_updates_v1(updates.iter().map(Vec::as_slice)).map_err(
            |err| {
                Error::new(ErrorKind::Internal, "cannot merge document updates")
                    .with_source(err)
            },
        ),
    }
}

/// Runs a storage call with exponential back-off while it fails with a
/// retryable error.
async fn retry<T, F, Fut>(
    deps: &Deps,
    op: &'static str,
    mut call: F,
) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut attempt = 0_u32;
    loop {
        let started = Instant::now();
        let result = call().await;
        deps.metrics
            .persist_ms
            .record(started.elapsed().as_secs_f64() * 1000.0, &label("op", op));
        match result {
            Ok(value) => return Ok(value),
            Err(err) => {
                deps.metrics.persist_errors.add(1, &label("op", op));
                attempt += 1;
                if attempt >= deps.config.persist_attempts
                    || !err.kind().is_retryable()
                {
                    return Err(err);
                }
                warn!(%err, attempt, op, "storage write failed; retrying");
                tokio::time::sleep(backoff(attempt)).await;
            }
        }
    }
}

fn backoff(attempt: u32) -> Duration {
    let millis = 50_u64.saturating_mul(1 << attempt.min(6));
    Duration::from_millis(millis.min(2_000))
}
