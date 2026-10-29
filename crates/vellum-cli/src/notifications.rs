// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Turns issue-tracker events into notifications.
//!
//! The Work service publishes its events to [`WorkEvents`] while it holds
//! its lock, so the sink only queues them. A background task takes them
//! one at a time, reads what it needs from the issue graph, maps handles
//! to accounts and hands each notification to the
//! [`Dispatcher`], which files inbox
//! lines and sends pushes.

use std::sync::Arc;

use jiff::Timestamp;
use tokio::sync::mpsc;
use vellum_identity::Identity;
use vellum_kernel::{Error, ErrorKind, UserId};
use vellum_notify::dispatch::Dispatcher;
use vellum_notify::endpoint::{SafeTransport, policy};
use vellum_notify::service::NotifyStore;
use vellum_notify::work::{IssueInfo, IssueLookup, notifications};
use vellum_work::{IssueId, WorkEvent, WorkGraph};
use vellum_work_service::{EventSink, WorkService};

/// Events waiting to be turned into notifications; past this, new ones
/// are dropped with a warning rather than holding up the Work service.
const QUEUE: usize = 4096;

/// The Work service's event sink: logs every event and queues it for
/// notifications.
#[derive(Debug)]
pub struct WorkEvents {
    queue: mpsc::Sender<WorkEvent>,
}

impl EventSink for WorkEvents {
    fn publish(&self, events: Vec<WorkEvent>) {
        for event in events {
            tracing::debug!(?event, "work event");
            if self.queue.try_send(event).is_err() {
                tracing::warn!("notification queue full; an event was not notified");
            }
        }
    }
}

/// The sink and the receiving end the notifier starts on.
#[must_use]
pub fn channel() -> (Arc<WorkEvents>, mpsc::Receiver<WorkEvent>) {
    let (queue, events) = mpsc::channel(QUEUE);
    (Arc::new(WorkEvents { queue }), events)
}

/// Starts the background task that notifies people about `events`.
///
/// # Errors
///
/// The outbound HTTP client cannot be built.
pub fn start(
    mut events: mpsc::Receiver<WorkEvent>,
    work: Arc<WorkService>,
    identity: Arc<Identity>,
    store: Arc<NotifyStore>,
) -> Result<(), Error> {
    let transport = SafeTransport::new(policy(&[])).map_err(|e| {
        Error::new(
            ErrorKind::Internal,
            format!("cannot build the push client: {e}"),
        )
    })?;
    let dispatcher = Dispatcher::new(store, transport);
    drop(tokio::spawn(async move {
        while let Some(event) = events.recv().await {
            notify(&event, &work, &identity, &dispatcher).await;
        }
    }));
    Ok(())
}

async fn notify<T: vellum_notify::delivery::HttpTransport>(
    event: &WorkEvent,
    work: &WorkService,
    identity: &Identity,
    dispatcher: &Dispatcher<T>,
) {
    let now = Timestamp::now();
    let addressed = work
        .read(|graph, _| notifications(event, &Graph(graph), now))
        .await;
    for one in addressed {
        let to: Vec<UserId> = one.to.iter().filter_map(|h| account(identity, h)).collect();
        if to.is_empty() {
            continue;
        }
        let actor = one
            .notification
            .actor
            .as_deref()
            .and_then(|h| account(identity, h));
        let _ = dispatcher.deliver(&one.notification, actor, &to, now).await;
    }
}

/// The account behind a handle; people unknown to sign-in (such as an
/// integration's bot) are not notified.
fn account(identity: &Identity, handle: &str) -> Option<UserId> {
    identity.user_by_handle(handle).map(|u| u.id)
}

struct Graph<'a>(&'a WorkGraph);

impl IssueLookup for Graph<'_> {
    fn issue(&self, id: IssueId) -> Option<IssueInfo> {
        self.0.issue(&id).map(|i| IssueInfo {
            key: i.key().to_string(),
            title: i.title.clone(),
            subscribers: i.subscribers.clone(),
        })
    }
}
