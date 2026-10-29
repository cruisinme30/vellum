// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Email delivery: the per-person queue, sending what fell due and signed
//! one-click unsubscribe links (ADR 0041, D174).
//!
//! The dispatcher queues an email for each planned email delivery
//! ([`queue`]). A timer calls [`Mailer::send_due`], which sends each
//! queued email once its time comes, unless by then:
//!
//! - the person read the inbox line about the same subject (the point of
//!   the email delay, D80);
//! - they muted the subject, switched email off for the kind, or snoozed
//!   (a snooze moves the email to the snooze's end).
//!
//! A burst about one subject collapses into one queued email carrying the
//! newest notification, sent at the earliest of their times. A failed send
//! is retried with backoff, five times at most.
//!
//! My Work's reminders and morning digest are notifications too (of the
//! `reminder` kind, sent at once), so they reach email the same way. Every
//! email carries an unsubscribe link, signed with a key kept in the
//! notify state, that mutes the subject without signing in.

use std::fmt::Write as _;
use std::sync::Arc;

use hmac::{Hmac, KeyInit as _, Mac as _};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tracing::Instrument as _;
use url::Url;
use vellum_kernel::{Error, ErrorKind, UserId};

use crate::email::{EmailMessage, MailTransport, Mailbox};
use crate::encoding::{b64url, hex};
use crate::inbox::{self, InboxEntry};
use crate::notification::{Channel, Notification, Preferences};
use crate::service::NotifyStore;

/// The most emails queued for one person; the oldest goes first.
pub const MAX_QUEUE: usize = 200;
/// Sends tried before an email is dropped.
pub const MAX_ATTEMPTS: u32 = 5;
/// The most subjects one person can mute (as in the preferences API).
const MAX_MUTED: usize = 1000;

/// An email waiting for its send time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingEmail {
    /// What it is about; the newest notification for the subject.
    pub notification: Notification,
    /// The earliest send time.
    pub not_before: Timestamp,
    /// Drop it if the inbox line about the subject is read by then.
    pub skip_if_read: bool,
    /// Failed sends so far.
    #[serde(default)]
    pub attempts: u32,
}

/// Queues an email about `notification`, collapsing it into one already
/// queued for the same subject.
pub fn queue(
    queue: &mut Vec<PendingEmail>,
    notification: &Notification,
    not_before: Timestamp,
    skip_if_read: bool,
) {
    if let Some(queued) = queue
        .iter_mut()
        .find(|q| q.notification.subject == notification.subject)
    {
        queued.notification = notification.clone();
        queued.not_before = queued.not_before.min(not_before);
        queued.skip_if_read &= skip_if_read;
        return;
    }
    queue.push(PendingEmail {
        notification: notification.clone(),
        not_before,
        skip_if_read,
        attempts: 0,
    });
    if queue.len() > MAX_QUEUE {
        let _ = queue.remove(0);
    }
}

/// A person to email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    /// The account.
    pub user: UserId,
    /// Their name and address.
    pub mailbox: Mailbox,
    /// Their handle, for reply addresses.
    pub handle: String,
}

/// Looks people up; the edge layer answers from sign-in.
pub trait Directory: Send + Sync {
    /// The person behind `user`, or `None` for a disabled or unknown
    /// account (who gets no email).
    fn contact(&self, user: UserId) -> Option<Contact>;
}

/// Makes the `Reply-To` address for an email about `subject`, so a reply
/// becomes a comment (the integrations crate checks it).
pub type ReplyAddress = Arc<dyn Fn(&str, &Contact) -> Option<String> + Send + Sync>;

/// How emails are addressed and linked.
#[derive(Clone)]
pub struct MailConfig {
    /// The sender, such as `Vellum <notifications@northstar.dev>`.
    pub from: Mailbox,
    /// The web app's public URL; links and unsubscribe URLs start here.
    pub base_url: Url,
    /// Reply addresses, when email replies are set up.
    pub reply_to: Option<ReplyAddress>,
}

impl std::fmt::Debug for MailConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MailConfig")
            .field("from", &self.from)
            .field("base_url", &self.base_url.as_str())
            .field("reply_to", &self.reply_to.is_some())
            .finish()
    }
}

/// What one pass over the queues did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MailReport {
    /// Emails the mail server accepted.
    pub sent: usize,
    /// Emails dropped: read meanwhile, muted, switched off or no address.
    pub skipped: usize,
    /// Emails that failed and will be tried again.
    pub retrying: usize,
    /// Emails that failed for the last time.
    pub failed: usize,
}

/// Sends queued notification emails.
pub struct Mailer<M> {
    store: Arc<NotifyStore>,
    transport: M,
    config: MailConfig,
}

impl<M> std::fmt::Debug for Mailer<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mailer")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// What to do with one queued email at send time.
enum Decision {
    Send,
    Skip(&'static str),
    Hold(Timestamp),
}

fn decide(
    pending: &PendingEmail,
    prefs: &Preferences,
    inbox: &[InboxEntry],
    now: Timestamp,
) -> Decision {
    let n = &pending.notification;
    let urgent = n.kind.is_urgent();
    if !prefs.channels_for(n.kind).contains(&Channel::Email) {
        return Decision::Skip("email switched off");
    }
    if !urgent && prefs.muted.contains(&n.subject) {
        return Decision::Skip("muted");
    }
    if pending.skip_if_read && !inbox::unread_about(inbox, &n.subject) {
        return Decision::Skip("read in the app");
    }
    match prefs.snoozed_until {
        Some(until) if !urgent && until > now => Decision::Hold(until),
        _ => Decision::Send,
    }
}

impl<M: MailTransport> Mailer<M> {
    /// A mailer over `store`, sending through `transport`.
    pub const fn new(store: Arc<NotifyStore>, transport: M, config: MailConfig) -> Self {
        Self {
            store,
            transport,
            config,
        }
    }

    /// The store.
    #[must_use]
    pub const fn store(&self) -> &Arc<NotifyStore> {
        &self.store
    }

    /// Sends every queued email of `users` whose time has come.
    pub async fn send_due(
        &self,
        users: &[UserId],
        directory: &dyn Directory,
        now: Timestamp,
    ) -> MailReport {
        let mut report = MailReport::default();
        for user in users {
            let due = self
                .store
                .update(*user, |state| {
                    let (due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut state.email_queue)
                        .into_iter()
                        .partition(|q| q.not_before <= now);
                    state.email_queue = later;
                    Ok(due)
                })
                .unwrap_or_default();
            if due.is_empty() {
                continue;
            }
            let state = self.store.user(*user);
            let contact = directory.contact(*user);
            let mut requeue = Vec::new();
            for mut pending in due {
                let Some(contact) = &contact else {
                    report.skipped += 1;
                    continue;
                };
                match decide(&pending, &state.preferences, &state.inbox, now) {
                    Decision::Skip(why) => {
                        tracing::debug!(why, "queued email dropped");
                        report.skipped += 1;
                    }
                    Decision::Hold(until) => {
                        pending.not_before = until;
                        requeue.push(pending);
                    }
                    Decision::Send => {
                        let message = self.notification_email(contact, &pending.notification, now);
                        if self.send(contact, &message, "notification").await {
                            report.sent += 1;
                        } else {
                            pending.attempts += 1;
                            if pending.attempts >= MAX_ATTEMPTS {
                                report.failed += 1;
                            } else {
                                pending.not_before =
                                    now.checked_add(backoff(pending.attempts)).unwrap_or(now);
                                report.retrying += 1;
                                requeue.push(pending);
                            }
                        }
                    }
                }
            }
            if !requeue.is_empty() {
                let saved = self.store.update(*user, |state| {
                    for pending in requeue {
                        queue(
                            &mut state.email_queue,
                            &pending.notification,
                            pending.not_before,
                            pending.skip_if_read,
                        );
                        if let Some(q) = state
                            .email_queue
                            .iter_mut()
                            .find(|q| q.notification.subject == pending.notification.subject)
                        {
                            q.attempts = q.attempts.max(pending.attempts);
                        }
                    }
                    Ok(())
                });
                if let Err(error) = saved {
                    tracing::warn!(%error, "email queue not saved");
                }
            }
        }
        if report != MailReport::default() {
            tracing::info!(
                sent = report.sent,
                skipped = report.skipped,
                retrying = report.retrying,
                failed = report.failed,
                "notification emails"
            );
        }
        report
    }

    async fn send(&self, contact: &Contact, message: &EmailMessage, kind: &'static str) -> bool {
        let bytes = match message.encode() {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(%error, kind, "email not built");
                return false;
            }
        };
        let span = tracing::info_span!(
            "notify.email",
            otel.kind = "client",
            kind,
            outcome = tracing::field::Empty
        );
        async {
            let result = self.transport.send(&contact.mailbox.address, &bytes).await;
            let outcome = if result.is_ok() { "delivered" } else { "retry" };
            let _ = tracing::Span::current().record("outcome", outcome);
            tracing::info!(
                monotonic_counter.vellum.notify.deliveries = 1_u64,
                channel = "email",
                outcome,
            );
            if let Err(error) = &result {
                tracing::warn!(%error, kind, "email not sent");
            }
            result.is_ok()
        }
        .instrument(span)
        .await
    }

    fn link(&self, path: &str) -> String {
        self.config
            .base_url
            .join(path)
            .map_or_else(|_| self.config.base_url.to_string(), |u| u.to_string())
    }

    fn unsubscribe_url(&self, user: UserId, subject: &str) -> String {
        let mut url = self
            .config
            .base_url
            .join("/unsubscribe")
            .unwrap_or_else(|_| self.config.base_url.clone());
        let _ = url
            .query_pairs_mut()
            .append_pair("u", &user.as_uuid().to_string())
            .append_pair("s", subject)
            .append_pair("t", &self.store.unsubscribe_token(user, subject));
        url.to_string()
    }

    fn envelope(
        &self,
        contact: &Contact,
        subject: String,
        thread: &str,
        now: Timestamp,
    ) -> EmailMessage {
        let domain = self
            .config
            .from
            .address
            .rsplit_once('@')
            .map_or("localhost", |(_, d)| d);
        EmailMessage {
            from: self.config.from.clone(),
            to: contact.mailbox.clone(),
            reply_to: None,
            subject,
            text: String::new(),
            html: String::new(),
            thread: thread_id(thread),
            message_id: format!("{}@{domain}", random_hex()),
            unsubscribe_url: String::new(),
            date: now,
        }
    }

    /// The email for one notification.
    #[must_use]
    pub fn notification_email(
        &self,
        contact: &Contact,
        n: &Notification,
        now: Timestamp,
    ) -> EmailMessage {
        let open = self.link(&n.url);
        let settings = self.link("/settings/notifications");
        let unsubscribe = self.unsubscribe_url(contact.user, &n.subject);
        let mut text = format!("{}\n\n", n.title);
        if !n.body.is_empty() {
            let _ = write!(text, "{}\n\n", n.body);
        }
        let _ = write!(
            text,
            "Open: {open}\n\n--\nStop emails about {subject}: {unsubscribe}\nNotification settings: {settings}\n",
            subject = n.subject
        );
        let html = page(&format!(
            "<p style=\"font-size:16px;font-weight:600;margin:0 0 12px\">{title}</p>{body}\
             <p><a href=\"{open}\" style=\"{BUTTON}\">Open {subject}</a></p>{footer}",
            title = esc(&n.title),
            body = if n.body.is_empty() {
                String::new()
            } else {
                format!(
                    "<p style=\"margin:0 0 16px;white-space:pre-wrap\">{}</p>",
                    esc(&n.body)
                )
            },
            open = esc(&open),
            subject = esc(&n.subject),
            footer = footer(
                &format!("Stop emails about {}", esc(&n.subject)),
                &unsubscribe,
                &settings
            ),
        ));
        let mut message = self.envelope(contact, n.title.clone(), &n.subject, now);
        message.text = text;
        message.html = html;
        message.unsubscribe_url = unsubscribe;
        message.reply_to = self
            .config
            .reply_to
            .as_ref()
            .and_then(|f| f(&n.subject, contact));
        message
    }
}

const BUTTON: &str = "display:inline-block;padding:8px 14px;border-radius:6px;background:#2f5bd3;color:#ffffff;text-decoration:none;font-weight:600";

fn page(inner: &str) -> String {
    format!(
        "<!doctype html><html><body style=\"margin:0;padding:24px;background:#f6f7f9;\">\
         <div style=\"max-width:560px;margin:0 auto;padding:24px;background:#ffffff;border-radius:8px;\
         font-family:-apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif;font-size:14px;line-height:1.5;color:#1d2330\">\
         {inner}</div></body></html>"
    )
}

fn footer(stop: &str, unsubscribe: &str, settings: &str) -> String {
    format!(
        "<hr style=\"border:0;border-top:1px solid #e3e6eb;margin:24px 0 12px\">\
         <p style=\"font-size:12px;color:#5f6b7a;margin:0\"><a href=\"{}\" style=\"color:#5f6b7a\">{stop}</a> · \
         <a href=\"{}\" style=\"color:#5f6b7a\">Notification settings</a></p>",
        esc(unsubscribe),
        esc(settings)
    )
}

/// HTML-escapes text for element content and quoted attributes.
fn esc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// A thread key safe for `Message-ID`-style headers.
fn thread_id(subject: &str) -> String {
    let cleaned: String = subject
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(100)
        .collect();
    if cleaned.is_empty() {
        "vellum".to_owned()
    } else {
        cleaned
    }
}

fn random_hex() -> String {
    let mut bytes = [0_u8; 16];
    // A zero ID on a failed RNG only weakens Message-ID uniqueness.
    let _ = getrandom::fill(&mut bytes);
    hex(&bytes)
}

/// The wait before retry number `attempt` (1-based): 2, 4, 8, 16 minutes.
fn backoff(attempt: u32) -> SignedDuration {
    SignedDuration::from_mins(1_i64 << attempt.clamp(1, 6))
}

/// The HMAC behind [`NotifyStore::unsubscribe_token`].
pub(crate) fn sign_unsubscribe(key: &[u8; 32], user: UserId, subject: &str) -> String {
    #[expect(clippy::expect_used, reason = "HMAC takes keys of any length")]
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
    mac.update(b"vellum-unsubscribe\0");
    mac.update(user.as_uuid().as_bytes());
    mac.update(b"\0");
    mac.update(subject.as_bytes());
    let tag = mac.finalize().into_bytes();
    b64url(tag.get(..16).unwrap_or_default())
}

/// Applies an unsubscribe link: mutes `subject` for `user`.
///
/// # Errors
///
/// A bad token, too many mutes, or state that cannot be saved.
pub fn unsubscribe(
    store: &NotifyStore,
    user: UserId,
    subject: &str,
    token: &str,
) -> Result<(), Error> {
    if subject.is_empty() || subject.len() > 200 || !store.check_unsubscribe(user, subject, token) {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "this unsubscribe link is not valid",
        ));
    }
    store.update(user, |state| {
        if !state.preferences.muted.contains(subject) {
            if state.preferences.muted.len() >= MAX_MUTED {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    "too many muted subjects",
                ));
            }
            let _ = state.preferences.muted.insert(subject.to_owned());
        }
        state
            .email_queue
            .retain(|q| q.notification.subject != subject);
        Ok(())
    })?;
    tracing::info!("unsubscribed from email about a subject");
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

    use std::sync::Mutex;

    use super::*;
    use crate::notification::NotificationKind;

    fn user() -> UserId {
        "01JC00000000000000000000A1".parse().unwrap()
    }

    fn at(minutes: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + minutes * 60).unwrap()
    }

    fn note(kind: NotificationKind, subject: &str, minute: i64) -> Notification {
        Notification {
            kind,
            subject: subject.to_owned(),
            title: format!("{subject} <b>changed</b>"),
            body: "Body & more".to_owned(),
            url: format!("/issues/{subject}"),
            actor: None,
            at: at(minute),
        }
    }

    #[derive(Default)]
    struct Outbox {
        sent: Mutex<Vec<(String, String)>>,
        fail: bool,
    }

    impl MailTransport for &Outbox {
        async fn send(&self, to: &str, message: &[u8]) -> Result<(), String> {
            if self.fail {
                return Err("421 try later".to_owned());
            }
            self.sent
                .lock()
                .unwrap()
                .push((to.to_owned(), String::from_utf8_lossy(message).into_owned()));
            Ok(())
        }
    }

    struct People;

    impl Directory for People {
        fn contact(&self, user: UserId) -> Option<Contact> {
            Some(Contact {
                user,
                mailbox: Mailbox {
                    name: "Zoë Park".to_owned(),
                    address: "zoe@northstar.dev".to_owned(),
                },
                handle: "zoe".to_owned(),
            })
        }
    }

    fn mailer(outbox: &Outbox) -> Mailer<&Outbox> {
        Mailer::new(
            Arc::new(NotifyStore::memory("https://northstar.dev").unwrap()),
            outbox,
            MailConfig {
                from: Mailbox {
                    name: "Vellum".to_owned(),
                    address: "notifications@northstar.dev".to_owned(),
                },
                base_url: Url::parse("https://vellum.northstar.dev").unwrap(),
                reply_to: None,
            },
        )
    }

    fn file(m: &Mailer<&Outbox>, n: &Notification, not_before: Timestamp, skip: bool) {
        m.store
            .update(user(), |s| {
                inbox::add(&mut s.inbox, n, "line".to_owned());
                queue(&mut s.email_queue, n, not_before, skip);
                Ok(())
            })
            .unwrap();
    }

    #[tokio::test]
    async fn sends_when_due_and_skips_what_was_read() {
        let outbox = Outbox::default();
        let m = mailer(&outbox);
        file(
            &m,
            &note(NotificationKind::Mentioned, "ENG-1", 0),
            at(10),
            true,
        );
        file(
            &m,
            &note(NotificationKind::Mentioned, "ENG-2", 0),
            at(10),
            true,
        );

        // Not yet due.
        assert_eq!(m.send_due(&[user()], &People, at(5)).await.sent, 0);
        // ENG-2 is read in the app before the delay ends.
        m.store
            .update(user(), |s| {
                for e in &mut s.inbox {
                    e.read = e.subject == "ENG-2";
                }
                Ok(())
            })
            .unwrap();
        let report = m.send_due(&[user()], &People, at(10)).await;
        assert_eq!((report.sent, report.skipped), (1, 1));
        let sent = outbox.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "zoe@northstar.dev");
        assert!(
            sent[0]
                .1
                .contains("List-Unsubscribe: <https://vellum.northstar.dev/unsubscribe?u=")
        );
        assert!(m.store.user(user()).email_queue.is_empty());
    }

    #[tokio::test]
    async fn a_burst_collapses_into_one_email_and_snooze_holds_it() {
        let outbox = Outbox::default();
        let m = mailer(&outbox);
        file(
            &m,
            &note(NotificationKind::Commented, "ENG-1", 0),
            at(10),
            true,
        );
        file(
            &m,
            &note(NotificationKind::Commented, "ENG-1", 3),
            at(13),
            true,
        );
        assert_eq!(m.store.user(user()).email_queue.len(), 1);
        assert_eq!(m.store.user(user()).email_queue[0].not_before, at(10));

        m.store
            .update(user(), |s| {
                s.preferences.snoozed_until = Some(at(60));
                Ok(())
            })
            .unwrap();
        assert_eq!(m.send_due(&[user()], &People, at(10)).await.sent, 0);
        assert_eq!(m.store.user(user()).email_queue[0].not_before, at(60));
        assert_eq!(m.send_due(&[user()], &People, at(60)).await.sent, 1);
    }

    #[tokio::test]
    async fn failures_retry_with_backoff_then_give_up() {
        let outbox = Outbox {
            fail: true,
            ..Outbox::default()
        };
        let m = mailer(&outbox);
        file(
            &m,
            &note(NotificationKind::IssueAssigned, "ENG-1", 0),
            at(0),
            false,
        );
        let mut now = at(0);
        for _ in 1..MAX_ATTEMPTS {
            assert_eq!(m.send_due(&[user()], &People, now).await.retrying, 1);
            now = m.store.user(user()).email_queue[0].not_before;
        }
        assert_eq!(m.send_due(&[user()], &People, now).await.failed, 1);
        assert!(m.store.user(user()).email_queue.is_empty());
    }

    #[tokio::test]
    async fn the_body_is_escaped_and_links_are_absolute() {
        let outbox = Outbox::default();
        let m = mailer(&outbox);
        let email = m.notification_email(
            &People.contact(user()).unwrap(),
            &note(NotificationKind::Mentioned, "ENG-7", 0),
            at(0),
        );
        assert!(email.html.contains("ENG-7 &lt;b&gt;changed&lt;/b&gt;"));
        assert!(email.html.contains("Body &amp; more"));
        assert!(!email.html.contains("<b>changed"));
        assert!(
            email
                .text
                .contains("Open: https://vellum.northstar.dev/issues/ENG-7")
        );
        assert_eq!(email.thread, "ENG-7");
        assert!(email.encode().is_ok());
    }

    #[test]
    fn unsubscribe_links_mute_only_with_a_valid_token() {
        let store = NotifyStore::memory("https://northstar.dev").unwrap();
        let token = store.unsubscribe_token(user(), "ENG-1");
        assert!(
            unsubscribe(&store, user(), "ENG-2", &token).is_err(),
            "other subject"
        );
        assert!(unsubscribe(&store, user(), "ENG-1", "AAAA").is_err());
        unsubscribe(&store, user(), "ENG-1", &token).unwrap();
        assert!(store.user(user()).preferences.muted.contains("ENG-1"));
    }
}
