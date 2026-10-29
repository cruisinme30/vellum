// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Delivers notifications: plans each one against the recipients'
//! preferences ([`crate::notification::plan`]), files the in-app inbox
//! lines and sends the web pushes.
//!
//! Pushes held by a snooze are not sent later; the inbox line is there
//! when the person looks. Emails are queued for [`crate::mail::Mailer`],
//! which sends them when their delay ends; without a mail server
//! configured they are counted and skipped.

use std::sync::Arc;

use jiff::Timestamp;
use vellum_kernel::UserId;

use crate::delivery::{self, HttpTransport, Outcome, PUSH_CHANNEL};
use crate::inbox;
use crate::notification::{Channel, Notification, Recipient, plan};
use crate::service::NotifyStore;
use crate::webpush::{self, Urgency};

/// What one delivery did, for logs and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// Inbox lines filed.
    pub inbox: usize,
    /// Pushes the push service accepted.
    pub pushed: usize,
    /// Pushes that failed or were refused.
    pub push_failed: usize,
    /// Pushes held by a snooze.
    pub held: usize,
    /// Emails queued for the mailer.
    pub email_queued: usize,
    /// Emails skipped for want of a mail transport.
    pub email_skipped: usize,
}

/// Delivers notifications through `transport`.
#[derive(Debug)]
pub struct Dispatcher<T> {
    store: Arc<NotifyStore>,
    transport: T,
    email: bool,
}

impl<T: HttpTransport> Dispatcher<T> {
    /// A dispatcher over `store`, sending pushes through `transport`.
    pub const fn new(store: Arc<NotifyStore>, transport: T) -> Self {
        Self {
            store,
            transport,
            email: false,
        }
    }

    /// Queues email deliveries for the mailer instead of skipping them;
    /// call it when a mail server is configured.
    #[must_use]
    pub const fn with_email(mut self) -> Self {
        self.email = true;
        self
    }

    /// Delivers `notification` to `to`; `actor`, who caused it, is never
    /// notified.
    pub async fn deliver(
        &self,
        notification: &Notification,
        actor: Option<UserId>,
        to: &[UserId],
        now: Timestamp,
    ) -> Report {
        let mut report = Report::default();
        let recipients: Vec<Recipient> = to
            .iter()
            .map(|u| Recipient {
                user: u.as_uuid().to_string(),
                preferences: self.store.user(*u).preferences,
            })
            .collect();
        // The planner compares the actor with recipients by user ID.
        let mut planned_for = notification.clone();
        planned_for.actor = actor.map(|a| a.as_uuid().to_string());
        for step in plan(&planned_for, &recipients) {
            let Some(user) = to
                .iter()
                .find(|u| u.as_uuid().to_string() == step.user)
                .copied()
            else {
                continue;
            };
            match step.channel {
                Channel::InApp => {
                    let filed = self.store.update(user, |state| {
                        inbox::add(&mut state.inbox, notification, random_id());
                        Ok(())
                    });
                    match filed {
                        Ok(()) => report.inbox += 1,
                        Err(error) => tracing::warn!(%error, "inbox line not saved"),
                    }
                }
                Channel::Push if step.not_before > now => report.held += 1,
                Channel::Push => self.push(user, notification, now, &mut report).await,
                Channel::Email if !self.email => report.email_skipped += 1,
                Channel::Email => {
                    let queued = self.store.update(user, |state| {
                        crate::mail::queue(
                            &mut state.email_queue,
                            notification,
                            step.not_before,
                            step.skip_if_read,
                        );
                        Ok(())
                    });
                    match queued {
                        Ok(()) => report.email_queued += 1,
                        Err(error) => tracing::warn!(%error, "email not queued"),
                    }
                }
            }
        }
        tracing::info!(
            kind = ?notification.kind,
            inbox = report.inbox,
            pushed = report.pushed,
            push_failed = report.push_failed,
            held = report.held,
            email_queued = report.email_queued,
            "notification delivered"
        );
        report
    }

    async fn push(
        &self,
        user: UserId,
        notification: &Notification,
        now: Timestamp,
        report: &mut Report,
    ) {
        let payload = serde_json::json!({
            "title": notification.title,
            "body": notification.body,
            "url": notification.url,
            "tag": notification.subject,
        })
        .to_string();
        let urgency = if notification.kind.is_urgent() {
            Urgency::High
        } else {
            Urgency::Normal
        };
        for stored in self.store.user(user).push {
            let request = stored.to_push().ok().and_then(|subscription| {
                webpush::prepare(
                    &subscription,
                    self.store.vapid(),
                    payload.as_bytes(),
                    &notification.subject,
                    urgency,
                    now,
                    random_id(),
                )
                .ok()
            });
            let Some(request) = request else {
                report.push_failed += 1;
                continue;
            };
            match delivery::attempt(&self.transport, &request, PUSH_CHANNEL, 1).await {
                Outcome::Delivered => report.pushed += 1,
                Outcome::Gone => {
                    report.push_failed += 1;
                    if let Err(error) = self.store.drop_endpoint(&stored.endpoint) {
                        tracing::warn!(%error, "expired push subscription not removed");
                    }
                }
                Outcome::Retry(_) | Outcome::Rejected => report.push_failed += 1,
            }
        }
    }
}

fn random_id() -> String {
    let mut bytes = [0_u8; 12];
    // An all-zero ID on a failed RNG only weakens uniqueness of inbox IDs.
    let _ = getrandom::fill(&mut bytes);
    bytes.iter().fold(String::with_capacity(24), |mut out, b| {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
        out
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

    use std::sync::Mutex;

    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    use super::*;
    use crate::delivery::{HttpResponse, TransportError};
    use crate::notification::NotificationKind;
    use crate::service::StoredSubscription;
    use crate::webhook::OutboundRequest;
    use crate::webpush::VapidKey;

    /// Answers each push with a fixed status and records where it went.
    struct Fake {
        status: u16,
        sent: Mutex<Vec<String>>,
    }

    impl HttpTransport for Fake {
        async fn post(&self, request: &OutboundRequest) -> Result<HttpResponse, TransportError> {
            self.sent.lock().unwrap().push(request.url.to_string());
            Ok(HttpResponse {
                status: self.status,
                retry_after: None,
            })
        }
    }

    fn id(n: char) -> UserId {
        format!("01JC00000000000000000000A{n}").parse().unwrap()
    }

    fn note(kind: NotificationKind) -> Notification {
        Notification {
            kind,
            subject: "ENG-1".to_owned(),
            title: "ENG-1 was assigned to you".to_owned(),
            body: "Fix login".to_owned(),
            url: "/issue/ENG-1".to_owned(),
            actor: None,
            at: Timestamp::UNIX_EPOCH,
        }
    }

    fn store_with_browser(user: UserId) -> Arc<NotifyStore> {
        let store = NotifyStore::memory("https://northstar.dev").unwrap();
        let browser = VapidKey::generate("x".to_owned()).unwrap();
        store
            .update(user, |s| {
                s.push.push(StoredSubscription {
                    endpoint: "https://push.example/sub/1".to_owned(),
                    p256dh: browser.public_key(),
                    auth: URL_SAFE_NO_PAD.encode([9_u8; 16]),
                    added: Timestamp::UNIX_EPOCH,
                });
                Ok(())
            })
            .unwrap();
        Arc::new(store)
    }

    #[tokio::test]
    async fn an_assignment_lands_in_the_inbox_and_on_the_phone() {
        let bo = id('1');
        let store = store_with_browser(bo);
        let fake = Fake {
            status: 201,
            sent: Mutex::new(Vec::new()),
        };
        let dispatcher = Dispatcher::new(Arc::clone(&store), fake);
        let report = dispatcher
            .deliver(
                &note(NotificationKind::IssueAssigned),
                Some(id('2')),
                &[bo],
                Timestamp::UNIX_EPOCH,
            )
            .await;
        assert_eq!(report.inbox, 1);
        assert_eq!(report.pushed, 1);
        assert_eq!(report.email_skipped, 1);
        assert_eq!(store.user(bo).inbox.len(), 1);
        assert_eq!(
            *dispatcher.transport.sent.lock().unwrap(),
            ["https://push.example/sub/1"]
        );

        // Nobody hears about their own action.
        let own = dispatcher
            .deliver(
                &note(NotificationKind::IssueAssigned),
                Some(bo),
                &[bo],
                Timestamp::UNIX_EPOCH,
            )
            .await;
        assert_eq!(own, Report::default());
    }

    #[tokio::test]
    async fn a_gone_browser_is_forgotten_and_snoozes_hold_pushes() {
        let bo = id('1');
        let store = store_with_browser(bo);
        let gone = Dispatcher::new(
            Arc::clone(&store),
            Fake {
                status: 410,
                sent: Mutex::new(Vec::new()),
            },
        );
        let report = gone
            .deliver(
                &note(NotificationKind::Mentioned),
                None,
                &[bo],
                Timestamp::UNIX_EPOCH,
            )
            .await;
        assert_eq!(report.push_failed, 1);
        assert!(store.user(bo).push.is_empty());

        let cy = id('3');
        let store = store_with_browser(cy);
        store
            .update(cy, |s| {
                s.preferences.snoozed_until = Some(Timestamp::from_second(3600).unwrap());
                Ok(())
            })
            .unwrap();
        let snoozed = Dispatcher::new(
            Arc::clone(&store),
            Fake {
                status: 201,
                sent: Mutex::new(Vec::new()),
            },
        );
        let report = snoozed
            .deliver(
                &note(NotificationKind::Mentioned),
                None,
                &[cy],
                Timestamp::UNIX_EPOCH,
            )
            .await;
        assert_eq!((report.inbox, report.held, report.pushed), (1, 1, 0));
    }
}
