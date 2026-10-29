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
//! A second task wakes every [`TICK`] and sends what fell due: webhook
//! deliveries and retries, queued emails, My Work reminders and each
//! person's daily digest (ADR 0041).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use tokio::sync::mpsc;
use vellum_identity::Identity;
use vellum_kernel::{Error, ErrorKind, UserId};
use vellum_notify::dispatch::Dispatcher;
use vellum_notify::endpoint::SafeTransport;
use vellum_notify::hooks::Webhooks;
use vellum_notify::mail::{Directory as _, Mailer, digest_due};
use vellum_notify::notification::{Notification, NotificationKind};
use vellum_notify::service::NotifyStore;
use vellum_notify::work::{IssueInfo, IssueLookup, notifications, webhook_events};
use vellum_tasks::remind;
use vellum_tasks::service::{NoInput, TasksService};
use vellum_work::{IssueId, WorkEvent, WorkGraph};
use vellum_work_service::{EventSink, WorkService};

use crate::mail::{Outbound, People};

/// Events waiting to be turned into notifications; past this, new ones
/// are dropped with a warning rather than holding up the Work service.
const QUEUE: usize = 4096;
/// How often due deliveries are sent.
pub const TICK: Duration = Duration::from_secs(30);
/// How far back reminders are caught up after a restart.
const REMINDER_CATCH_UP: SignedDuration = SignedDuration::from_hours(1);

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

/// What the notifier delivers through besides the inbox and push.
pub struct Delivery {
    /// Webhook subscriptions and their queue.
    pub webhooks: Arc<Webhooks<SafeTransport>>,
    /// Email, when a mail server is configured.
    pub mail: Option<Arc<Mailer<Outbound>>>,
    /// People to email, and the workspace time zone.
    pub people: Arc<People>,
    /// My Work, for reminders and the digest.
    pub tasks: Arc<TasksService>,
    /// The workspace time zone, for reminders.
    pub time_zone: jiff::tz::TimeZone,
}

/// Starts the background tasks that notify people about `events` and
/// send what falls due.
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
) -> Result<(), Error> {
    let transport = SafeTransport::new(vellum_notify::endpoint::policy(&[])).map_err(|e| {
        Error::new(
            ErrorKind::Internal,
            format!("cannot build the push client: {e}"),
        )
    })?;
    let mut dispatcher = Dispatcher::new(Arc::clone(&store), transport);
    if delivery.mail.is_some() {
        dispatcher = dispatcher.with_email();
    }
    let dispatcher = Arc::new(dispatcher);
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
    drop(tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let _ = tick.tick().await;
            run_due(&delivery, &store, &dispatcher, Timestamp::now()).await;
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

/// Queues the webhook deliveries `event` causes.
async fn publish(event: &WorkEvent, work: &WorkService, webhooks: &Webhooks<SafeTransport>) {
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
async fn run_due<T: vellum_notify::delivery::HttpTransport>(
    delivery: &Delivery,
    store: &NotifyStore,
    dispatcher: &Dispatcher<T>,
    now: Timestamp,
) {
    let _ = delivery.webhooks.run_due(now).await;
    let people = delivery.people.everyone();
    reminders(delivery, store, dispatcher, &people, now).await;
    if let Some(mailer) = &delivery.mail {
        let _ = mailer
            .send_due(&people, delivery.people.as_ref(), now)
            .await;
        digests(mailer, delivery, &people, now).await;
    }
}

/// Notifies each person about My Work reminders that fired since the
/// last pass (R-MW-5).
async fn reminders<T: vellum_notify::delivery::HttpTransport>(
    delivery: &Delivery,
    store: &NotifyStore,
    dispatcher: &Dispatcher<T>,
    people: &[UserId],
    now: Timestamp,
) {
    let settings = remind::Settings::new(delivery.time_zone.clone());
    let floor = now.checked_sub(REMINDER_CATCH_UP).unwrap_or(now);
    for user in people {
        let after = store
            .user(*user)
            .reminders_until
            .map_or(floor, |t| t.max(floor));
        if after >= now {
            continue;
        }
        let due = match delivery
            .tasks
            .reminders_due(*user, &settings, after, now)
            .await
        {
            Ok(due) => due,
            Err(error) => {
                tracing::debug!(%error, "reminders skipped");
                continue;
            }
        };
        for reminder in due {
            let note = Notification {
                kind: NotificationKind::Reminder,
                subject: reminder.task.clone(),
                title: format!("Reminder: {}", reminder.title),
                body: String::new(),
                url: "/my-work".to_owned(),
                actor: None,
                at: reminder.at,
            };
            let _ = dispatcher.deliver(&note, None, &[*user], now).await;
        }
        if let Err(error) = store.update(*user, |s| {
            s.reminders_until = Some(now);
            Ok(())
        }) {
            tracing::warn!(%error, "reminder progress not saved");
        }
    }
}

/// Sends each person's daily digest when it falls due.
async fn digests(
    mailer: &Mailer<Outbound>,
    delivery: &Delivery,
    people: &[UserId],
    now: Timestamp,
) {
    for user in people {
        let Some(contact) = delivery.people.contact(*user) else {
            continue;
        };
        let state = mailer.store().user(*user);
        if digest_due(
            &state.preferences,
            state.digest_sent_on,
            &contact.time_zone,
            now,
        )
        .is_none()
        {
            continue;
        }
        let my_work = delivery
            .tasks
            .digest(*user, NoInput {})
            .await
            .ok()
            .and_then(|d| d.markdown);
        if let Err(error) = mailer.send_digest(&contact, my_work.as_deref(), now).await {
            tracing::warn!(%error, "digest not sent");
        }
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
