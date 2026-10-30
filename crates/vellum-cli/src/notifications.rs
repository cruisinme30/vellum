// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Turns issue-tracker events into notifications and webhook deliveries,
//! and runs the delivery timer.
//!
//! The Work service publishes its events to [`WorkEvents`] while it holds
//! its lock, so the sink only queues them. A background task takes them
//! one at a time, reads what it needs from the issue graph, maps handles
//! to accounts and hands each notification to the [`Dispatcher`] (inbox,
//! push, email queue). The same event is turned into webhook events and
//! queued for every matching hook.
//!
//! A [`Ticker`] pass, run every [`TICK`] on one replica of the cluster,
//! sends what fell due: webhook deliveries and retries, and queued emails
//! (ADR 0041). My Work's reminders and digest are sent by its own
//! scheduler as notifications, so they are emailed through the same queue.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use tokio::sync::mpsc;
use vellum_identity::Identity;
use vellum_kernel::{Error, ErrorKind, UserId};
use vellum_notify::dispatch::Dispatcher;
use vellum_notify::endpoint::SafeTransport;
use vellum_notify::hooks::Webhooks;
use vellum_notify::mail::Mailer;
use vellum_notify::service::NotifyStore;
use vellum_notify::work::{
    IssueInfo, IssueLookup, notifications, webhook_events,
};
use vellum_work::{IssueId, WorkEvent, WorkGraph};
use vellum_work_service::{EventSink, WorkService};

use crate::mail::{Outbound, People};

/// Events waiting to be turned into notifications; past this, new ones
/// are dropped with a warning rather than holding up the Work service.
const QUEUE: usize = 4096;
/// How often due deliveries are sent.
pub const TICK: Duration = Duration::from_secs(30);

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
                tracing::warn!(
                    "notification queue full; an event was not notified"
                );
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

/// What the notifier delivers through besides the inbox and push.
pub struct Delivery {
    /// Webhook subscriptions and their queue.
    pub webhooks: Arc<Webhooks<SafeTransport>>,
    /// Email, when a mail server is configured.
    pub mail: Option<Arc<Mailer<Outbound>>>,
    /// People to email.
    pub people: Arc<People>,
}

/// One pass of the delivery timer, cheap to clone into a cluster loop.
#[derive(Clone)]
pub struct Ticker {
    delivery: Arc<Delivery>,
}

impl Ticker {
    /// Sends what fell due: webhook deliveries and queued emails.
    pub async fn run(&self) {
        run_due(&self.delivery, Timestamp::now()).await;
    }
}

/// Starts the background task that notifies people about `events`, and
/// returns the timer pass for the caller to run every [`TICK`] on one
/// replica of the cluster.
///
/// # Errors
///
/// The outbound HTTP client cannot be built.
pub fn start(
    mut events: mpsc::Receiver<WorkEvent>,
    work: Arc<WorkService>,
    identity: Arc<Identity>,
    store: Arc<NotifyStore>,
    delivery: Delivery,
) -> Result<Ticker, Error> {
    let transport = SafeTransport::new(vellum_notify::endpoint::policy(&[]))
        .map_err(|e| {
            Error::new(
                ErrorKind::Internal,
                format!("cannot build the push client: {e}"),
            )
        })?;
    let dispatcher = Arc::new(Dispatcher::new(store, transport));
    let webhooks = Arc::clone(&delivery.webhooks);
    {
        let dispatcher = Arc::clone(&dispatcher);
        drop(tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                notify(&event, &work, &identity, &dispatcher).await;
                publish(&event, &work, &webhooks).await;
            }
        }));
    }
    Ok(Ticker {
        delivery: Arc::new(delivery),
    })
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
        let to: Vec<UserId> =
            one.to.iter().filter_map(|h| account(identity, h)).collect();
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

/// Queues the webhook deliveries `event` causes.
async fn publish(
    event: &WorkEvent,
    work: &WorkService,
    webhooks: &Webhooks<SafeTransport>,
) {
    if !webhooks.store().any_enabled() {
        return;
    }
    let now = Timestamp::now();
    let hooked = work
        .read(|graph, _| webhook_events(event, &Graph(graph), now))
        .await;
    if !hooked.is_empty() {
        // Work teams have no private flag yet, so every team is public to
        // hooks that cover all public teams.
        let _ = webhooks.publish(&hooked, &BTreeSet::new(), now);
    }
}

/// One pass of the delivery timer.
async fn run_due(delivery: &Delivery, now: Timestamp) {
    let _ = delivery.webhooks.run_due(now).await;
    if let Some(mailer) = &delivery.mail {
        let people = delivery.people.everyone();
        let _ = mailer
            .send_due(&people, delivery.people.as_ref(), now)
            .await;
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
            team: i.team.to_string(),
            subscribers: i.subscribers.clone(),
        })
    }
}
