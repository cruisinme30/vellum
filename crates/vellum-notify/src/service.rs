// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The notifications service: each person's web push subscriptions and
//! notification preferences, behind the gateway.
//!
//! | Operation | Route | Does |
//! |---|---|---|
//! | `notify.push_key` | `GET /v1/push/vapid-key` | the server's VAPID public key, for `applicationServerKey` |
//! | `notify.push_subscribe` | `POST /v1/push/subscriptions` | registers this browser (`PushSubscription.toJSON()`) |
//! | `notify.push_unsubscribe` | `DELETE /v1/push/subscriptions?endpoint=` | forgets this browser |
//! | `notify.preferences` | `GET /v1/notifications/preferences` | your channels per kind, mutes and snooze |
//! | `notify.set_preferences` | `PUT /v1/notifications/preferences` | replaces them |
//! | `notify.inbox` | `GET /v1/notifications` | your inbox, newest first, and the unread count |
//! | `notify.read` | `POST /v1/notifications/read` | marks lines (or all) read |
//!
//! Everything is the caller's own: no operation takes a user ID. State
//! lives in `.vellum/notify.json` in the data folder, written atomically
//! and readable only by the server's user. The VAPID key is made once, on
//! the first start, and kept there, so browsers stay subscribed across
//! restarts.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::{SignedDuration, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;
use vellum_contracts::{HttpMethod, Operation, RequestContext, Scope, Service};
use vellum_kernel::{Error, ErrorKind, UserId};

use crate::inbox::{self, InboxEntry, InboxItem};
use crate::mail::PendingEmail;
use crate::notification::{Channel, NotificationKind, Preferences};
use crate::webpush::{PushSubscription, VapidKey};

/// The most browsers one person can register; the oldest goes first.
pub const MAX_SUBSCRIPTIONS: usize = 20;
/// The longest push endpoint URL accepted.
const MAX_ENDPOINT: usize = 2048;
/// The most subjects one person can mute.
const MAX_MUTED: usize = 1000;
/// The longest email delay a person can choose.
const MAX_EMAIL_DELAY: SignedDuration = SignedDuration::from_hours(24);

/// One registered browser, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSubscription {
    /// The push service URL.
    pub endpoint: String,
    /// `keys.p256dh`, base64url.
    pub p256dh: String,
    /// `keys.auth`, base64url.
    pub auth: String,
    /// When it was registered or last refreshed.
    pub added: Timestamp,
}

impl StoredSubscription {
    /// The subscription in the form [`crate::webpush`] encrypts to.
    ///
    /// # Errors
    ///
    /// A stored value that no longer parses (edited by hand).
    pub fn to_push(&self) -> Result<PushSubscription, Error> {
        let endpoint = Url::parse(&self.endpoint).map_err(|e| invalid(e.to_string()))?;
        PushSubscription::from_browser(endpoint, &self.p256dh, &self.auth)
            .map_err(|e| invalid(e.to_string()))
    }
}

/// One person's stored state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserState {
    /// Notification settings.
    pub preferences: Preferences,
    /// Registered browsers, oldest first.
    pub push: Vec<StoredSubscription>,
    /// The in-app inbox, newest first.
    pub inbox: Vec<InboxEntry>,
    /// Emails waiting for their send time ([`crate::mail`]).
    pub email_queue: Vec<PendingEmail>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct State {
    /// The VAPID secret, base64url.
    vapid: Option<String>,
    /// The key that signs unsubscribe links, base64url.
    mail_key: Option<String>,
    /// Per user, by user ID.
    users: BTreeMap<String, UserState>,
}

/// Where the service keeps its state.
pub struct NotifyStore {
    path: Option<PathBuf>,
    state: Mutex<State>,
    vapid: VapidKey,
    mail_key: [u8; 32],
    /// Whether a mail server is configured, so email deliveries queue.
    email: std::sync::atomic::AtomicBool,
}

impl std::fmt::Debug for NotifyStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotifyStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl NotifyStore {
    /// A store kept only in memory, for tests and `vellum contracts`.
    ///
    /// # Errors
    ///
    /// The system random source fails.
    pub fn memory(subject: &str) -> Result<Self, Error> {
        let vapid = VapidKey::generate(subject.to_owned()).map_err(internal)?;
        Ok(Self {
            path: None,
            state: Mutex::new(State::default()),
            vapid,
            mail_key: random_key()?,
            email: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Opens `<data_dir>/.vellum/notify.json`, making the VAPID key on
    /// the first start. `subject` is the contact push services see: the
    /// workspace's public URL or a `mailto:` address.
    ///
    /// # Errors
    ///
    /// An unreadable or invalid file, or a file that cannot be written.
    pub fn open(data_dir: &Path, subject: &str) -> Result<Self, Error> {
        let path = data_dir.join(".vellum").join("notify.json");
        let mut state: State = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| internal(format!("cannot parse {}: {e}", path.display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => return Err(internal(format!("cannot read {}: {e}", path.display()))),
        };
        let vapid = if let Some(secret) = &state.vapid {
            let bytes = URL_SAFE_NO_PAD
                .decode(secret)
                .map_err(|_| internal(format!("{}: bad VAPID key", path.display())))?;
            VapidKey::from_bytes(&bytes, subject.to_owned()).map_err(internal)?
        } else {
            let key = VapidKey::generate(subject.to_owned()).map_err(internal)?;
            state.vapid = Some(URL_SAFE_NO_PAD.encode(key.secret_bytes()));
            save(&path, &state)?;
            tracing::info!("web push key created");
            key
        };
        let mail_key = match state.mail_key.as_deref().map(|k| URL_SAFE_NO_PAD.decode(k)) {
            Some(Ok(bytes)) => <[u8; 32]>::try_from(bytes.as_slice())
                .map_err(|_| internal(format!("{}: bad mail key", path.display())))?,
            Some(Err(_)) => return Err(internal(format!("{}: bad mail key", path.display()))),
            None => {
                let key = random_key()?;
                state.mail_key = Some(URL_SAFE_NO_PAD.encode(key));
                save(&path, &state)?;
                key
            }
        };
        Ok(Self {
            path: Some(path),
            state: Mutex::new(state),
            vapid,
            mail_key,
            email: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// The server's VAPID key, for signing pushes.
    #[must_use]
    pub const fn vapid(&self) -> &VapidKey {
        &self.vapid
    }

    /// Turns email on: from now on every dispatcher over this store
    /// queues email deliveries for the mailer. The server calls it when a
    /// mail server is configured.
    pub fn enable_email(&self) {
        self.email.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Whether email deliveries are queued.
    #[must_use]
    pub fn email_enabled(&self) -> bool {
        self.email.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The signature that lets the link in an email unsubscribe `user`
    /// from `subject` without signing in (RFC 8058).
    #[must_use]
    pub fn unsubscribe_token(&self, user: UserId, subject: &str) -> String {
        crate::mail::sign_unsubscribe(&self.mail_key, user, subject)
    }

    /// Whether `token` is [`Self::unsubscribe_token`] for `user` and
    /// `subject`, compared in constant time.
    #[must_use]
    pub fn check_unsubscribe(&self, user: UserId, subject: &str, token: &str) -> bool {
        use subtle::ConstantTimeEq as _;
        let expected = self.unsubscribe_token(user, subject);
        expected.len() == token.len() && bool::from(expected.as_bytes().ct_eq(token.as_bytes()))
    }

    /// Everyone with stored state, as user IDs.
    #[must_use]
    pub fn known_users(&self) -> Vec<UserId> {
        self.lock()
            .users
            .keys()
            .filter_map(|k| k.parse::<uuid::Uuid>().ok())
            .map(UserId::from_uuid)
            .collect()
    }

    /// A person's state (defaults when they have none).
    #[must_use]
    pub fn user(&self, user: UserId) -> UserState {
        self.lock()
            .users
            .get(&key(user))
            .cloned()
            .unwrap_or_default()
    }

    /// Changes a person's state and saves. The change stays in memory if
    /// saving fails, and the error is returned.
    ///
    /// # Errors
    ///
    /// `f`'s refusal, or a file that cannot be written.
    pub fn update<R>(
        &self,
        user: UserId,
        f: impl FnOnce(&mut UserState) -> Result<R, Error>,
    ) -> Result<R, Error> {
        let mut state = self.lock();
        let entry = state.users.entry(key(user)).or_default();
        let mut next = entry.clone();
        let out = f(&mut next)?;
        *entry = next;
        if let Some(path) = &self.path {
            save(path, &state)?;
        }
        drop(state);
        Ok(out)
    }

    /// Forgets one browser of anyone, after the push service answered
    /// 404 or 410 for it.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn drop_endpoint(&self, endpoint: &str) -> Result<(), Error> {
        let mut state = self.lock();
        let mut changed = false;
        for user in state.users.values_mut() {
            let before = user.push.len();
            user.push.retain(|s| s.endpoint != endpoint);
            changed |= user.push.len() != before;
        }
        if changed && let Some(path) = &self.path {
            save(path, &state)?;
        }
        drop(state);
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn random_key() -> Result<[u8; 32], Error> {
    let mut key = [0_u8; 32];
    getrandom::fill(&mut key).map_err(|e| internal(format!("no random source: {e}")))?;
    Ok(key)
}

fn key(user: UserId) -> String {
    user.as_uuid().to_string()
}

fn save(path: &Path, state: &State) -> Result<(), Error> {
    save_json(path, state)
}

/// Writes `value` as JSON to `path` atomically, readable only by the
/// server's user (the files hold secrets).
pub(crate) fn save_json<T: Serialize>(path: &Path, value: &T) -> Result<(), Error> {
    let fail = |e: std::io::Error| internal(format!("cannot write {}: {e}", path.display()));
    let bytes = serde_json::to_vec(value).map_err(|e| internal(e.to_string()))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    let _ = options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        // The VAPID secret is in here.
        let _ = options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(fail)?;
    file.write_all(&bytes).map_err(fail)?;
    file.sync_all().map_err(fail)?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(fail)
}

fn internal(e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Internal, e.to_string())
}

fn invalid(e: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, e.into())
}

/// No input.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Empty {}

/// The VAPID public key.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PushKey {
    /// Base64url, uncompressed P-256; pass it as `applicationServerKey`.
    pub public_key: String,
}

/// The browser's keys.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PushKeys {
    /// The browser's P-256 public key, base64url.
    pub p256dh: String,
    /// The 16-byte auth secret, base64url.
    pub auth: String,
}

/// `PushSubscription.toJSON()`.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Subscribe {
    /// The push service URL (`https` only).
    pub endpoint: String,
    /// When the browser will drop the subscription; not used.
    #[serde(default)]
    pub expiration_time: Option<f64>,
    /// The keys messages are encrypted to.
    pub keys: PushKeys,
}

/// Which browser to forget.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Unsubscribe {
    /// The push service URL the browser registered.
    pub endpoint: String,
}

/// The result of a push registration change.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PushDevices {
    /// How many browsers you have registered now.
    pub devices: usize,
}

/// Notification settings, as the API shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PreferencesView {
    /// Channels (`in_app`, `email`, `push`) per kind. Every kind is
    /// listed; on input, a kind left out keeps its defaults.
    pub channels: BTreeMap<String, BTreeSet<String>>,
    /// Minutes email waits for the inbox entry to be read (0 to 1440).
    pub email_delay_minutes: i64,
    /// Issue keys and document IDs you muted.
    pub muted: BTreeSet<String>,
    /// Non-urgent push and email are held until this time (RFC 3339).
    pub snoozed_until: Option<String>,
}

const fn kind_name(kind: NotificationKind) -> &'static str {
    match kind {
        NotificationKind::IssueAssigned => "issue_assigned",
        NotificationKind::Mentioned => "mentioned",
        NotificationKind::Commented => "commented",
        NotificationKind::StateChanged => "state_changed",
        NotificationKind::TriageNew => "triage_new",
        NotificationKind::SlaHighRisk => "sla_high_risk",
        NotificationKind::SlaBreached => "sla_breached",
        NotificationKind::ReviewRequested => "review_requested",
        NotificationKind::ProjectUpdate => "project_update",
        NotificationKind::Reminder => "reminder",
        NotificationKind::PullRequest => "pull_request",
    }
}

const fn channel_name(channel: Channel) -> &'static str {
    match channel {
        Channel::InApp => "in_app",
        Channel::Email => "email",
        Channel::Push => "push",
    }
}

impl From<&Preferences> for PreferencesView {
    fn from(p: &Preferences) -> Self {
        Self {
            channels: NotificationKind::ALL
                .iter()
                .map(|k| {
                    let on = p.channels_for(*k).into_iter().map(channel_name);
                    (kind_name(*k).to_owned(), on.map(str::to_owned).collect())
                })
                .collect(),
            email_delay_minutes: p.email_delay.as_mins(),
            muted: p.muted.clone(),
            snoozed_until: p.snoozed_until.map(|t| t.to_string()),
        }
    }
}

impl PreferencesView {
    /// The stored form.
    ///
    /// # Errors
    ///
    /// An unknown kind or channel, a delay out of range, too many mutes or
    /// a bad time.
    pub fn to_preferences(&self) -> Result<Preferences, Error> {
        let mut channels = BTreeMap::new();
        for (name, on) in &self.channels {
            let kind = NotificationKind::ALL
                .into_iter()
                .find(|k| kind_name(*k) == name)
                .ok_or_else(|| invalid(format!("unknown notification kind {name:?}")))?;
            let mut set = BTreeSet::new();
            for channel in on {
                let channel = [Channel::InApp, Channel::Email, Channel::Push]
                    .into_iter()
                    .find(|c| channel_name(*c) == channel)
                    .ok_or_else(|| invalid(format!("unknown channel {channel:?}")))?;
                let _ = set.insert(channel);
            }
            if set != kind.default_channels() {
                let _ = channels.insert(kind, set);
            }
        }
        let email_delay = SignedDuration::from_mins(self.email_delay_minutes);
        if email_delay.is_negative() || email_delay > MAX_EMAIL_DELAY {
            return Err(invalid("email_delay_minutes must be 0 to 1440"));
        }
        if self.muted.len() > MAX_MUTED || self.muted.iter().any(|m| m.len() > 200) {
            return Err(invalid(format!(
                "at most {MAX_MUTED} muted subjects of 200 characters"
            )));
        }
        let snoozed_until = self
            .snoozed_until
            .as_deref()
            .map(str::parse::<Timestamp>)
            .transpose()
            .map_err(|e| invalid(format!("snoozed_until: {e}")))?;
        Ok(Preferences {
            channels,
            email_delay,
            muted: self.muted.clone(),
            snoozed_until,
        })
    }
}

/// Which inbox lines to list.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct InboxQuery {
    /// Only unread lines.
    #[serde(default)]
    pub unread_only: bool,
}

/// Your inbox.
#[derive(Debug, Serialize, JsonSchema)]
pub struct InboxPage {
    /// Lines, newest first.
    pub items: Vec<InboxItem>,
    /// How many are unread.
    pub unread: usize,
}

/// Which lines to mark read.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MarkRead {
    /// Line IDs.
    #[serde(default)]
    pub ids: Vec<String>,
    /// Every line instead.
    #[serde(default)]
    pub all: bool,
}

/// The unread count after a change.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Unread {
    /// How many lines are unread.
    pub unread: usize,
}

/// The notifications service.
#[derive(Debug, Clone)]
pub struct NotifyService {
    store: Arc<NotifyStore>,
}

impl NotifyService {
    /// The service over `store`.
    #[must_use]
    pub const fn new(store: Arc<NotifyStore>) -> Self {
        Self { store }
    }

    /// Registers a browser for `user`, replacing an earlier registration
    /// of the same endpoint and dropping the oldest past the limit.
    fn subscribe(&self, user: UserId, input: &Subscribe) -> Result<PushDevices, Error> {
        let new = subscription(input, Timestamp::now())?;
        let devices = self.store.update(user, |state| {
            state.push.retain(|old| old.endpoint != new.endpoint);
            state.push.push(new);
            while state.push.len() > MAX_SUBSCRIPTIONS {
                let _ = state.push.remove(0);
            }
            Ok(state.push.len())
        })?;
        tracing::info!(devices, "push subscription registered");
        Ok(PushDevices { devices })
    }

    /// Forgets one of `user`'s browsers.
    fn unsubscribe(&self, user: UserId, input: &Unsubscribe) -> Result<PushDevices, Error> {
        // Stored endpoints are normalized by the URL parser.
        let wanted =
            Url::parse(&input.endpoint).map_or_else(|_| input.endpoint.clone(), |u| u.to_string());
        let devices = self.store.update(user, |state| {
            state.push.retain(|old| old.endpoint != wanted);
            Ok(state.push.len())
        })?;
        Ok(PushDevices { devices })
    }

    /// The store, for the delivery side.
    #[must_use]
    pub const fn store(&self) -> &Arc<NotifyStore> {
        &self.store
    }
}

/// Checks and normalizes a browser registration.
fn subscription(input: &Subscribe, now: Timestamp) -> Result<StoredSubscription, Error> {
    if input.endpoint.len() > MAX_ENDPOINT {
        return Err(invalid("the push endpoint is too long"));
    }
    let endpoint = Url::parse(&input.endpoint).map_err(|e| invalid(format!("endpoint: {e}")))?;
    // Push services are public HTTPS hosts; anything else would make the
    // server send requests where a browser told it to.
    if endpoint.scheme() != "https" || endpoint.host_str().is_none() {
        return Err(invalid("the push endpoint must be an https URL"));
    }
    if !endpoint.username().is_empty() || endpoint.password().is_some() {
        return Err(invalid("the push endpoint must not carry credentials"));
    }
    let _ = PushSubscription::from_browser(endpoint.clone(), &input.keys.p256dh, &input.keys.auth)
        .map_err(|e| invalid(e.to_string()))?;
    Ok(StoredSubscription {
        endpoint: endpoint.to_string(),
        p256dh: input.keys.p256dh.clone(),
        auth: input.keys.auth.clone(),
        added: now,
    })
}

fn inbox_operations(store: &Arc<NotifyStore>) -> Vec<Operation> {
    let list = {
        let store = Arc::clone(store);
        move |ctx: RequestContext, input: InboxQuery| {
            let store = Arc::clone(&store);
            async move {
                let user = ctx.principal.require_user()?;
                let state = store.user(user);
                let unread = state.inbox.iter().filter(|e| !e.read).count();
                let items = state
                    .inbox
                    .iter()
                    .filter(|e| !input.unread_only || !e.read)
                    .map(InboxItem::from)
                    .collect();
                Ok(InboxPage { items, unread })
            }
        }
    };
    let read = {
        let store = Arc::clone(store);
        move |ctx: RequestContext, input: MarkRead| {
            let store = Arc::clone(&store);
            async move {
                let user = ctx.principal.require_user()?;
                let unread = store.update(user, |state| {
                    let _ = inbox::mark_read(&mut state.inbox, &input.ids, input.all);
                    Ok(state.inbox.iter().filter(|e| !e.read).count())
                })?;
                Ok(Unread { unread })
            }
        }
    };
    vec![
        Operation::query("notify.inbox", "/v1/notifications", Scope::Read, list)
            .summary("Your notifications, newest first"),
        Operation::mutation(
            "notify.read",
            HttpMethod::Post,
            "/v1/notifications/read",
            Scope::Read,
            read,
        )
        .summary("Mark notifications read"),
    ]
}

impl Service for NotifyService {
    fn name(&self) -> &'static str {
        "notify"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let key = {
            let s = Arc::clone(&self);
            move |_ctx: RequestContext, _: Empty| {
                let public_key = s.store.vapid().public_key();
                async move { Ok(PushKey { public_key }) }
            }
        };
        let subscribe = {
            let s = Arc::clone(&self);
            move |ctx: RequestContext, input: Subscribe| {
                let s = Arc::clone(&s);
                async move { s.subscribe(ctx.principal.require_user()?, &input) }
            }
        };
        let unsubscribe = {
            let s = Arc::clone(&self);
            move |ctx: RequestContext, input: Unsubscribe| {
                let s = Arc::clone(&s);
                async move { s.unsubscribe(ctx.principal.require_user()?, &input) }
            }
        };
        let preferences = {
            let s = Arc::clone(&self);
            move |ctx: RequestContext, _: Empty| {
                let s = Arc::clone(&s);
                async move {
                    let user = ctx.principal.require_user()?;
                    Ok(PreferencesView::from(&s.store.user(user).preferences))
                }
            }
        };
        let store = Arc::clone(&self.store);
        let set_preferences = {
            let s = self;
            move |ctx: RequestContext, input: PreferencesView| {
                let s = Arc::clone(&s);
                async move {
                    let user = ctx.principal.require_user()?;
                    let next = input.to_preferences()?;
                    let view = PreferencesView::from(&next);
                    s.store.update(user, |state| {
                        state.preferences = next;
                        Ok(())
                    })?;
                    Ok(view)
                }
            }
        };
        let mut ops = vec![
            Operation::query("notify.push_key", "/v1/push/vapid-key", Scope::Read, key)
                .summary("The server's web push key, for subscribing a browser"),
            Operation::mutation(
                "notify.push_subscribe",
                HttpMethod::Post,
                "/v1/push/subscriptions",
                Scope::Read,
                subscribe,
            )
            .summary("Register this browser for push notifications"),
            Operation::mutation(
                "notify.push_unsubscribe",
                HttpMethod::Delete,
                "/v1/push/subscriptions",
                Scope::Read,
                unsubscribe,
            )
            .summary("Stop push notifications to one browser"),
            Operation::query(
                "notify.preferences",
                "/v1/notifications/preferences",
                Scope::Read,
                preferences,
            )
            .summary("Your notification channels, mutes and snooze"),
            Operation::mutation(
                "notify.set_preferences",
                HttpMethod::Put,
                "/v1/notifications/preferences",
                Scope::Read,
                set_preferences,
            )
            .summary("Change your notification channels, mutes and snooze"),
        ];
        ops.extend(inbox_operations(&store));
        ops
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

    use super::*;

    fn user() -> UserId {
        "01JC00000000000000000000A1".parse().unwrap()
    }

    fn browser(endpoint: &str) -> Subscribe {
        // Any P-256 public key will do; the VAPID key is one.
        let key = VapidKey::generate("https://northstar.dev".to_owned()).unwrap();
        Subscribe {
            endpoint: endpoint.to_owned(),
            expiration_time: None,
            keys: PushKeys {
                p256dh: key.public_key(),
                auth: URL_SAFE_NO_PAD.encode([7_u8; 16]),
            },
        }
    }

    #[test]
    fn only_https_push_services_with_valid_keys_are_accepted() {
        let now = Timestamp::UNIX_EPOCH;
        assert!(subscription(&browser("https://fcm.googleapis.com/fcm/send/abc"), now).is_ok());
        for bad in [
            "http://fcm.googleapis.com/x",
            "file:///etc/passwd",
            "https://user:pw@push.example/x",
            "not a url",
        ] {
            assert!(subscription(&browser(bad), now).is_err(), "{bad}");
        }
        let mut short = browser("https://push.example/x");
        short.keys.auth = URL_SAFE_NO_PAD.encode([1_u8; 8]);
        assert!(subscription(&short, now).is_err());
        let long = format!("https://push.example/{}", "a".repeat(MAX_ENDPOINT));
        assert!(subscription(&browser(&long), now).is_err());
    }

    #[test]
    fn preferences_round_trip_and_reject_nonsense() {
        let defaults = PreferencesView::from(&Preferences::default());
        assert_eq!(defaults.channels.len(), NotificationKind::ALL.len());
        assert_eq!(defaults.email_delay_minutes, 10);
        assert_eq!(defaults.to_preferences().unwrap(), Preferences::default());

        let mut view = defaults.clone();
        let _ = view
            .channels
            .insert("commented".to_owned(), BTreeSet::from(["push".to_owned()]));
        let _ = view.muted.insert("ENG-1".to_owned());
        view.snoozed_until = Some("2026-11-02T09:00:00Z".to_owned());
        let stored = view.to_preferences().unwrap();
        assert_eq!(
            stored.channels_for(NotificationKind::Commented),
            BTreeSet::from([Channel::Push])
        );
        assert_eq!(stored.channels.len(), 1, "defaults are not stored");
        assert_eq!(PreferencesView::from(&stored), view);

        for broken in [
            {
                let mut v = defaults.clone();
                let _ = v.channels.insert("party".to_owned(), BTreeSet::new());
                v
            },
            {
                let mut v = defaults.clone();
                let _ = v
                    .channels
                    .insert("mentioned".to_owned(), BTreeSet::from(["sms".to_owned()]));
                v
            },
            PreferencesView {
                email_delay_minutes: -1,
                ..defaults.clone()
            },
            PreferencesView {
                email_delay_minutes: 1441,
                ..defaults.clone()
            },
            PreferencesView {
                snoozed_until: Some("tomorrow".to_owned()),
                ..defaults
            },
        ] {
            assert!(broken.to_preferences().is_err(), "{broken:?}");
        }
    }

    #[test]
    fn state_and_the_push_key_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = NotifyStore::open(dir.path(), "https://northstar.dev").unwrap();
        let public = store.vapid().public_key();
        let sub = subscription(&browser("https://push.example/a"), Timestamp::UNIX_EPOCH).unwrap();
        store
            .update(user(), |s| {
                s.push.push(sub.clone());
                Ok(())
            })
            .unwrap();

        let again = NotifyStore::open(dir.path(), "https://northstar.dev").unwrap();
        assert_eq!(again.vapid().public_key(), public);
        assert_eq!(again.user(user()).push, vec![sub.clone()]);
        assert!(sub.to_push().is_ok());

        // A gone browser is forgotten for everyone.
        again.drop_endpoint("https://push.example/a").unwrap();
        assert!(again.user(user()).push.is_empty());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.path().join(".vellum/notify.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "only the server's user can read the key");
        }
    }

    #[test]
    fn a_refused_change_leaves_state_alone() {
        let store = NotifyStore::memory("https://northstar.dev").unwrap();
        let failed = store.update(user(), |s| {
            let _ = s.preferences.muted.insert("ENG-1".to_owned());
            Err::<(), _>(invalid("no"))
        });
        assert!(failed.is_err());
        assert!(store.user(user()).preferences.muted.is_empty());
    }
}
