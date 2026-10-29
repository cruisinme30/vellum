// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Webhook subscriptions an admin manages, and their delivery (#149,
//! ADR 0041, D218).
//!
//! | Operation | Route | Does |
//! |---|---|---|
//! | `webhooks.list` | `GET /v1/admin/webhooks` | every hook, with its health and last delivery |
//! | `webhooks.create` | `POST /v1/admin/webhooks` | adds a hook; the signing secret is shown once |
//! | `webhooks.update` | `PUT /v1/admin/webhooks/{id}` | changes the URL, events, teams or on/off |
//! | `webhooks.delete` | `DELETE /v1/admin/webhooks/{id}` | removes a hook and its queued deliveries |
//! | `webhooks.rotate_secret` | `POST /v1/admin/webhooks/{id}/secret` | a new secret; the old one still signs for 24 hours |
//! | `webhooks.test` | `POST /v1/admin/webhooks/{id}/test` | sends a signed `ping` now and shows the answer |
//! | `webhooks.deliveries` | `GET /v1/admin/webhooks/{id}/deliveries` | the last 100 attempts |
//! | `webhooks.redeliver` | `POST /v1/admin/webhooks/{id}/redeliver` | sends one logged event again |
//!
//! Every operation needs the `admin` scope. A URL must pass the SSRF
//! policy ([`crate::endpoint`]) when it is saved, and the transport checks
//! the resolved address again at connect time.
//!
//! [`Webhooks::publish`] queues one delivery per matching hook;
//! [`Webhooks::run_due`], on a timer, sends what is due. A failed attempt
//! is retried on the [`RetryPolicy`] (about 45 hours), and a hook that
//! keeps failing for three days is paused and shows why. State lives in
//! `.vellum/webhooks.json`, readable only by the server's user because it
//! holds the signing secrets.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use jiff::{SignedDuration, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;
use vellum_contracts::{HttpMethod, Operation, RequestContext, Scope, Service};
use vellum_kernel::{Error, ErrorKind};
use vellum_security::ssrf::UrlPolicy;

use crate::delivery::{
    self, EndpointHealth, HealthAction, HttpTransport, Outcome, RetryPolicy, WEBHOOK_CHANNEL,
};
use crate::endpoint::check_url;
use crate::event::{EventAction, EventType, WebhookEvent};
use crate::secret::WebhookSecret;
use crate::service::save_json;
use crate::webhook::{DELIVERY_HEADER, EVENT_HEADER, Subscription, TeamScope, prepare};

/// The most hooks a workspace can have.
pub const MAX_HOOKS: usize = 50;
/// Attempts kept in each hook's log.
pub const LOG_PER_HOOK: usize = 100;
/// The most deliveries waiting; past this the oldest are dropped.
pub const MAX_PENDING: usize = 10_000;
/// Deliveries sent in one pass of [`Webhooks::run_due`].
pub const BATCH: usize = 200;
/// How long a rotated-out secret keeps signing.
pub const ROTATION_GRACE: SignedDuration = SignedDuration::from_hours(24);
/// The longest description.
const MAX_DESCRIPTION: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSecret {
    /// `whsec_…`.
    value: String,
    /// When a rotated-out secret stops signing.
    retires_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredHook {
    id: String,
    url: String,
    #[serde(default)]
    description: String,
    event_types: BTreeSet<EventType>,
    teams: TeamScope,
    enabled: bool,
    #[serde(default)]
    paused_reason: Option<String>,
    secrets: Vec<StoredSecret>,
    #[serde(default)]
    health: EndpointHealth,
    created_at: Timestamp,
    #[serde(default)]
    created_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Logged {
    view: DeliveryView,
    /// The event, so it can be sent again.
    event: Option<WebhookEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pending {
    delivery_id: String,
    hook: String,
    event: WebhookEvent,
    attempt: usize,
    due: Timestamp,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct HookState {
    hooks: Vec<StoredHook>,
    log: BTreeMap<String, Vec<Logged>>,
    pending: Vec<Pending>,
}

/// Where hooks, their logs and queued deliveries are kept.
pub struct HookStore {
    path: Option<PathBuf>,
    state: Mutex<HookState>,
    policy: UrlPolicy,
}

impl std::fmt::Debug for HookStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookStore")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

fn internal(e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Internal, e.to_string())
}

fn invalid(e: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, e.into())
}

fn not_found() -> Error {
    Error::new(ErrorKind::NotFound, "no such webhook")
}

impl HookStore {
    /// A store kept only in memory, checking URLs against `policy`.
    #[must_use]
    pub fn memory(policy: UrlPolicy) -> Self {
        Self {
            path: None,
            state: Mutex::new(HookState::default()),
            policy,
        }
    }

    /// Opens `<data_dir>/.vellum/webhooks.json`.
    ///
    /// # Errors
    ///
    /// An unreadable or invalid file.
    pub fn open(data_dir: &Path, policy: UrlPolicy) -> Result<Self, Error> {
        let path = data_dir.join(".vellum").join("webhooks.json");
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| internal(format!("cannot parse {}: {e}", path.display())))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HookState::default(),
            Err(e) => return Err(internal(format!("cannot read {}: {e}", path.display()))),
        };
        Ok(Self {
            path: Some(path),
            state: Mutex::new(state),
            policy,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HookState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn change<R>(&self, f: impl FnOnce(&mut HookState) -> Result<R, Error>) -> Result<R, Error> {
        let mut state = self.lock();
        let out = f(&mut state)?;
        if let Some(path) = &self.path {
            save_json(path, &*state)?;
        }
        drop(state);
        Ok(out)
    }

    /// Whether any hook is on (callers skip building events otherwise).
    #[must_use]
    pub fn any_enabled(&self) -> bool {
        self.lock().hooks.iter().any(|h| h.enabled)
    }

    /// Takes up to [`BATCH`] deliveries due at `now` off the queue.
    fn take_due(&self, now: Timestamp) -> Vec<Pending> {
        let mut state = self.lock();
        let (mut due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut state.pending)
            .into_iter()
            .partition(|p| p.due <= now);
        let rest = due.split_off(due.len().min(BATCH));
        state.pending = rest.into_iter().chain(later).collect();
        drop(state);
        due
    }

    /// How many deliveries are waiting.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.lock().pending.len()
    }
}

fn subscription(hook: &StoredHook, now: Timestamp) -> Result<Subscription, Error> {
    let url = Url::parse(&hook.url).map_err(internal)?;
    let secrets = hook
        .secrets
        .iter()
        .filter(|s| s.retires_at.is_none_or(|t| t > now))
        .map(|s| WebhookSecret::parse(&s.value).map_err(internal))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Subscription {
        id: hook.id.clone(),
        url,
        secrets,
        event_types: hook.event_types.clone(),
        teams: hook.teams.clone(),
        enabled: hook.enabled,
    })
}

fn random_id(prefix: &str) -> String {
    let mut bytes = [0_u8; 10];
    let _ = getrandom::fill(&mut bytes);
    let mut out = prefix.to_owned();
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn event_name(event: &WebhookEvent) -> String {
    format!("{}.{}", event.event_type.as_str(), event.action.as_str())
}

/// What one pass of [`Webhooks::run_due`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HookReport {
    /// Deliveries the receiver accepted.
    pub delivered: usize,
    /// Failed attempts that will be retried.
    pub retrying: usize,
    /// Deliveries that failed for good.
    pub failed: usize,
    /// Hooks paused for failing too long.
    pub paused: usize,
}

/// Delivers webhook events through `transport`.
#[derive(Debug)]
pub struct Webhooks<T> {
    store: Arc<HookStore>,
    transport: T,
    retry: RetryPolicy,
}

impl<T: HttpTransport> Webhooks<T> {
    /// Delivery over `store` through `transport`, with the default retries.
    pub fn new(store: Arc<HookStore>, transport: T) -> Self {
        Self {
            store,
            transport,
            retry: RetryPolicy::default(),
        }
    }

    /// The store.
    #[must_use]
    pub const fn store(&self) -> &Arc<HookStore> {
        &self.store
    }

    /// Queues `events` for every hook that wants them. `private_teams`
    /// are the keys of private teams, which only a hook naming them hears
    /// about. Returns how many deliveries were queued.
    pub fn publish(
        &self,
        events: &[WebhookEvent],
        private_teams: &BTreeSet<String>,
        now: Timestamp,
    ) -> usize {
        let queued = self.store.change(|state| {
            let mut queued = 0;
            for event in events {
                if let Err(error) = event.validate() {
                    tracing::warn!(%error, "webhook event dropped");
                    continue;
                }
                for hook in &state.hooks {
                    let Ok(sub) = subscription(hook, now) else {
                        continue;
                    };
                    if sub.matches(event, private_teams) {
                        state.pending.push(Pending {
                            delivery_id: format!("{}.{}", event.id, hook.id),
                            hook: hook.id.clone(),
                            event: event.clone(),
                            attempt: 0,
                            due: now,
                        });
                        queued += 1;
                    }
                }
            }
            let over = state.pending.len().saturating_sub(MAX_PENDING);
            if over > 0 {
                tracing::warn!(
                    dropped = over,
                    "webhook queue full; oldest deliveries dropped"
                );
                let _ = state.pending.drain(..over);
            }
            Ok(queued)
        });
        queued.unwrap_or_else(|error| {
            tracing::warn!(%error, "webhook deliveries not saved");
            0
        })
    }

    /// Sends the deliveries that are due, at most [`BATCH`] per call.
    pub async fn run_due(&self, now: Timestamp) -> HookReport {
        let mut report = HookReport::default();
        let due = self.store.take_due(now);
        for pending in due {
            let hook = self
                .store
                .lock()
                .hooks
                .iter()
                .find(|h| h.id == pending.hook)
                .cloned();
            let Some(hook) = hook.filter(|h| h.enabled) else {
                continue;
            };
            let (view, outcome) = self.send(&hook, &pending, now, false).await;
            let next = self.retry.next_attempt(pending.attempt, outcome, now);
            let paused = self.store.change(|state| {
                let mut paused = false;
                if let Some(next) = next {
                    state.pending.push(Pending {
                        attempt: pending.attempt + 1,
                        due: next,
                        ..pending.clone()
                    });
                }
                if let Some(h) = state.hooks.iter_mut().find(|h| h.id == hook.id) {
                    // Only final outcomes count towards pausing.
                    if next.is_none() && h.health.record(outcome, now) == HealthAction::Pause {
                        h.enabled = false;
                        h.paused_reason = Some(format!(
                            "paused after {} failed deliveries since {}",
                            h.health.consecutive_failures,
                            h.health
                                .failing_since
                                .map(|t| t.to_string())
                                .unwrap_or_default()
                        ));
                        state.pending.retain(|p| p.hook != h.id);
                        paused = true;
                    }
                }
                let mut view = view.clone();
                view.next_attempt = next.map(|t| t.to_string());
                push_log(state, &hook.id, view, Some(pending.event.clone()));
                Ok(paused)
            });
            match (outcome, next) {
                (Outcome::Delivered, _) => report.delivered += 1,
                (_, Some(_)) => report.retrying += 1,
                (_, None) => report.failed += 1,
            }
            if paused.unwrap_or(false) {
                tracing::warn!(hook = %hook.id, "webhook paused after failing for days");
                report.paused += 1;
            }
        }
        if report != HookReport::default() {
            tracing::info!(
                delivered = report.delivered,
                retrying = report.retrying,
                failed = report.failed,
                paused = report.paused,
                "webhook deliveries"
            );
        }
        report
    }

    async fn send(
        &self,
        hook: &StoredHook,
        pending: &Pending,
        now: Timestamp,
        ping: bool,
    ) -> (DeliveryView, Outcome) {
        let request = subscription(hook, now)
            .and_then(|sub| prepare(&pending.event, &sub, now).map_err(internal))
            .map(|mut request| {
                request.delivery_id.clone_from(&pending.delivery_id);
                for (name, value) in &mut request.headers {
                    if name == DELIVERY_HEADER {
                        value.clone_from(&pending.delivery_id);
                    } else if name == EVENT_HEADER && ping {
                        "ping".clone_into(value);
                    }
                }
                request
            });
        let mut view = DeliveryView {
            delivery_id: pending.delivery_id.clone(),
            event_id: pending.event.id.clone(),
            event: if ping {
                "ping".to_owned()
            } else {
                event_name(&pending.event)
            },
            attempt: pending.attempt + 1,
            at: now.to_string(),
            status: None,
            outcome: String::new(),
            error: None,
            duration_ms: 0,
            next_attempt: None,
        };
        let outcome = match request {
            Ok(request) => {
                let done = delivery::attempt_detailed(
                    &self.transport,
                    &request,
                    WEBHOOK_CHANNEL,
                    pending.attempt + 1,
                )
                .await;
                view.status = done.status;
                view.error = done.error;
                view.duration_ms = done.elapsed_ms;
                done.outcome
            }
            Err(error) => {
                view.error = Some(error.to_string());
                Outcome::Rejected
            }
        };
        outcome.as_str().clone_into(&mut view.outcome);
        (view, outcome)
    }

    /// Sends a signed `ping` to a hook now, even a paused one, and logs it.
    ///
    /// # Errors
    ///
    /// An unknown hook.
    pub async fn ping(&self, id: &str, now: Timestamp) -> Result<DeliveryView, Error> {
        let hook = self
            .store
            .lock()
            .hooks
            .iter()
            .find(|h| h.id == id)
            .cloned()
            .ok_or_else(not_found)?;
        let event_type = hook
            .event_types
            .iter()
            .next()
            .copied()
            .unwrap_or(EventType::Issue);
        let action = if event_type == EventType::IssueSla {
            EventAction::SlaSet
        } else {
            EventAction::Update
        };
        let event = WebhookEvent {
            id: random_id("ping_"),
            event_type,
            action,
            occurred_at: now,
            team: None,
            actor: None,
            data: json!({"ping": true, "message": "Vellum test delivery"}),
            updated_from: None,
            url: None,
        };
        let mut sendable = hook.clone();
        sendable.enabled = true;
        let pending = Pending {
            delivery_id: format!("{}.{}", event.id, hook.id),
            hook: hook.id.clone(),
            event,
            attempt: 0,
            due: now,
        };
        let (view, _) = self.send(&sendable, &pending, now, true).await;
        let logged = view.clone();
        self.store.change(|state| {
            push_log(state, &hook.id, logged, None);
            Ok(())
        })?;
        Ok(view)
    }

    /// Queues a logged delivery's event again, as a new delivery.
    ///
    /// # Errors
    ///
    /// An unknown hook or delivery, or a ping (which has no event).
    pub fn redeliver(&self, id: &str, delivery_id: &str, now: Timestamp) -> Result<(), Error> {
        self.store.change(|state| {
            if !state.hooks.iter().any(|h| h.id == id) {
                return Err(not_found());
            }
            let event = state
                .log
                .get(id)
                .and_then(|log| log.iter().find(|l| l.view.delivery_id == delivery_id))
                .and_then(|l| l.event.clone())
                .ok_or_else(|| Error::new(ErrorKind::NotFound, "no such delivery to send again"))?;
            state.pending.push(Pending {
                delivery_id: random_id("redeliver_"),
                hook: id.to_owned(),
                event,
                attempt: 0,
                due: now,
            });
            Ok(())
        })
    }
}

fn push_log(state: &mut HookState, hook: &str, view: DeliveryView, event: Option<WebhookEvent>) {
    let log = state.log.entry(hook.to_owned()).or_default();
    log.insert(0, Logged { view, event });
    log.truncate(LOG_PER_HOOK);
}

// --- The admin API -------------------------------------------------------

/// One delivery attempt, as the admin sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeliveryView {
    /// The delivery ID, as sent in `Vellum-Delivery`.
    pub delivery_id: String,
    /// The event ID, as in the payload's `id`.
    pub event_id: String,
    /// `issue.create`, `comment.update`, … or `ping`.
    pub event: String,
    /// 1 for the first attempt.
    pub attempt: usize,
    /// When it was sent (RFC 3339).
    pub at: String,
    /// The receiver's status code, when it answered.
    pub status: Option<u16>,
    /// `delivered`, `retry`, `rejected` or `gone`.
    pub outcome: String,
    /// Why there was no answer.
    pub error: Option<String>,
    /// How long it took.
    pub duration_ms: u64,
    /// When it will be tried again (RFC 3339).
    pub next_attempt: Option<String>,
}

/// A webhook, as the admin sees it. The secret is never shown again.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct HookView {
    /// The hook's ID.
    pub id: String,
    /// Where events are posted.
    pub url: String,
    /// A note for admins.
    pub description: String,
    /// The resources it wants (`issue`, `comment`, …).
    pub event_types: Vec<String>,
    /// The team keys it covers; empty means every public team.
    pub teams: Vec<String>,
    /// Whether it receives events.
    pub enabled: bool,
    /// Why Vellum paused it, when it did.
    pub paused_reason: Option<String>,
    /// The last four characters of the current secret.
    pub secret_hint: String,
    /// Whether a rotated-out secret still signs too.
    pub rotating: bool,
    /// Final failures in a row.
    pub consecutive_failures: u32,
    /// When the current run of failures began (RFC 3339).
    pub failing_since: Option<String>,
    /// The latest attempt.
    pub last_delivery: Option<DeliveryView>,
    /// Deliveries waiting.
    pub queued: usize,
    /// When it was created (RFC 3339).
    pub created_at: String,
}

fn view(state: &HookState, hook: &StoredHook, now: Timestamp) -> HookView {
    let current = hook.secrets.first().map_or("", |s| s.value.as_str());
    HookView {
        id: hook.id.clone(),
        url: hook.url.clone(),
        description: hook.description.clone(),
        event_types: hook
            .event_types
            .iter()
            .map(|t| t.as_str().to_owned())
            .collect(),
        teams: match &hook.teams {
            TeamScope::AllPublic => Vec::new(),
            TeamScope::Only(keys) => keys.iter().cloned().collect(),
        },
        enabled: hook.enabled,
        paused_reason: hook.paused_reason.clone(),
        secret_hint: current
            .get(current.len().saturating_sub(4)..)
            .unwrap_or("")
            .to_owned(),
        rotating: hook
            .secrets
            .iter()
            .skip(1)
            .any(|s| s.retires_at.is_some_and(|t| t > now)),
        consecutive_failures: hook.health.consecutive_failures,
        failing_since: hook.health.failing_since.map(|t| t.to_string()),
        last_delivery: state
            .log
            .get(&hook.id)
            .and_then(|l| l.first())
            .map(|l| l.view.clone()),
        queued: state.pending.iter().filter(|p| p.hook == hook.id).count(),
        created_at: hook.created_at.to_string(),
    }
}

/// No input.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct NoInput {}

/// A hook by ID.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HookId {
    /// The hook's ID.
    pub id: String,
}

/// A logged delivery of a hook.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RedeliverInput {
    /// The hook's ID.
    pub id: String,
    /// The delivery to send again.
    pub delivery_id: String,
}

/// A new hook.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct HookInput {
    /// Where to post events: `https`, port 443 or 8443, a public address.
    pub url: String,
    /// A note for admins.
    #[serde(default)]
    pub description: String,
    /// The resources to send: `issue`, `comment`, `project`,
    /// `project_update`, `cycle`, `label`, `user`, `issue_sla`.
    pub event_types: Vec<String>,
    /// Team keys to cover, private ones included; empty for every public
    /// team.
    #[serde(default)]
    pub teams: Vec<String>,
}

/// A hook's new settings; everything is replaced.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct HookUpdate {
    /// The hook's ID.
    pub id: String,
    /// The settings.
    #[serde(flatten)]
    pub hook: HookInput,
    /// On or off. Turning a paused hook on clears its failures.
    pub enabled: bool,
}

/// The hooks.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HookList {
    /// Every hook, oldest first.
    pub hooks: Vec<HookView>,
    /// The resources a hook can want.
    pub event_types: Vec<String>,
}

/// A hook with its secret, shown once.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HookCreated {
    /// The hook.
    pub hook: HookView,
    /// The signing secret (`whsec_…`). Store it now: it is not shown again.
    pub secret: String,
}

/// A hook's recent attempts.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DeliveryList {
    /// Newest first.
    pub deliveries: Vec<DeliveryView>,
}

/// Done.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Done {
    /// Always true.
    pub ok: bool,
}

struct Checked {
    url: Url,
    description: String,
    event_types: BTreeSet<EventType>,
    teams: TeamScope,
}

fn check(input: &HookInput, policy: &UrlPolicy) -> Result<Checked, Error> {
    if input.url.len() > 2048 {
        return Err(invalid("the URL is too long"));
    }
    let url = check_url(input.url.trim(), policy).map_err(|e| invalid(format!("url: {e}")))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(invalid("url: put credentials in the receiver, not the URL"));
    }
    if input.description.chars().count() > MAX_DESCRIPTION {
        return Err(invalid(format!(
            "the description is longer than {MAX_DESCRIPTION} characters"
        )));
    }
    let event_types = input
        .event_types
        .iter()
        .map(|t| t.parse::<EventType>().map_err(|e| invalid(e.to_string())))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if event_types.is_empty() {
        return Err(invalid("choose at least one event type"));
    }
    let mut keys = BTreeSet::new();
    for team in &input.teams {
        let key = team.trim().to_ascii_uppercase();
        if key.is_empty() || key.len() > 10 || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(invalid(format!("{team:?} is not a team key")));
        }
        let _ = keys.insert(key);
    }
    Ok(Checked {
        url,
        description: input.description.trim().to_owned(),
        event_types,
        teams: if keys.is_empty() {
            TeamScope::AllPublic
        } else {
            TeamScope::Only(keys)
        },
    })
}

fn new_secret() -> Result<String, Error> {
    WebhookSecret::generate()
        .map(|s| s.reveal())
        .map_err(internal)
}

impl HookStore {
    /// Every hook.
    #[must_use]
    pub fn list(&self, now: Timestamp) -> HookList {
        let state = self.lock();
        HookList {
            hooks: state.hooks.iter().map(|h| view(&state, h, now)).collect(),
            event_types: EventType::ALL
                .iter()
                .map(|t| t.as_str().to_owned())
                .collect(),
        }
    }

    /// Adds a hook.
    ///
    /// # Errors
    ///
    /// A URL the policy refuses, unknown event types, bad team keys or too
    /// many hooks.
    pub fn create(
        &self,
        input: &HookInput,
        by: Option<String>,
        now: Timestamp,
    ) -> Result<HookCreated, Error> {
        let checked = check(input, &self.policy)?;
        let secret = new_secret()?;
        let hook = StoredHook {
            id: random_id("wh_"),
            url: checked.url.to_string(),
            description: checked.description,
            event_types: checked.event_types,
            teams: checked.teams,
            enabled: true,
            paused_reason: None,
            secrets: vec![StoredSecret {
                value: secret.clone(),
                retires_at: None,
            }],
            health: EndpointHealth::default(),
            created_at: now,
            created_by: by,
        };
        let created = self.change(|state| {
            if state.hooks.len() >= MAX_HOOKS {
                return Err(invalid(format!("at most {MAX_HOOKS} webhooks")));
            }
            state.hooks.push(hook.clone());
            Ok(view(state, &hook, now))
        })?;
        tracing::info!(hook = %created.id, host = checked.url.host_str().unwrap_or_default(), "webhook created");
        Ok(HookCreated {
            hook: created,
            secret,
        })
    }

    /// Replaces a hook's settings.
    ///
    /// # Errors
    ///
    /// As [`Self::create`], or an unknown hook.
    pub fn update(&self, input: &HookUpdate, now: Timestamp) -> Result<HookView, Error> {
        let checked = check(&input.hook, &self.policy)?;
        self.change(|state| {
            let hook = state
                .hooks
                .iter_mut()
                .find(|h| h.id == input.id)
                .ok_or_else(not_found)?;
            hook.url = checked.url.to_string();
            hook.description = checked.description;
            hook.event_types = checked.event_types;
            hook.teams = checked.teams;
            if input.enabled && !hook.enabled {
                hook.health = EndpointHealth::default();
                hook.paused_reason = None;
            }
            hook.enabled = input.enabled;
            let hook = hook.clone();
            if !hook.enabled {
                state.pending.retain(|p| p.hook != hook.id);
            }
            Ok(view(state, &hook, now))
        })
    }

    /// Removes a hook, its log and its queued deliveries.
    ///
    /// # Errors
    ///
    /// An unknown hook.
    pub fn delete(&self, id: &str) -> Result<(), Error> {
        self.change(|state| {
            let before = state.hooks.len();
            state.hooks.retain(|h| h.id != id);
            if state.hooks.len() == before {
                return Err(not_found());
            }
            let _ = state.log.remove(id);
            state.pending.retain(|p| p.hook != id);
            Ok(())
        })?;
        tracing::info!(hook = %id, "webhook deleted");
        Ok(())
    }

    /// Makes a new secret; the previous one keeps signing (as a second
    /// `v1` signature) for [`ROTATION_GRACE`].
    ///
    /// # Errors
    ///
    /// An unknown hook.
    pub fn rotate(&self, id: &str, now: Timestamp) -> Result<String, Error> {
        let secret = new_secret()?;
        let retires = now.checked_add(ROTATION_GRACE).unwrap_or(now);
        self.change(|state| {
            let hook = state
                .hooks
                .iter_mut()
                .find(|h| h.id == id)
                .ok_or_else(not_found)?;
            hook.secrets.retain(|s| s.retires_at.is_none());
            for old in &mut hook.secrets {
                old.retires_at = Some(retires);
            }
            hook.secrets.insert(
                0,
                StoredSecret {
                    value: secret.clone(),
                    retires_at: None,
                },
            );
            hook.secrets.truncate(2);
            Ok(())
        })?;
        tracing::info!(hook = %id, "webhook secret rotated");
        Ok(secret)
    }

    /// A hook's recent attempts, newest first.
    ///
    /// # Errors
    ///
    /// An unknown hook.
    pub fn deliveries(&self, id: &str) -> Result<DeliveryList, Error> {
        let state = self.lock();
        if !state.hooks.iter().any(|h| h.id == id) {
            return Err(not_found());
        }
        Ok(DeliveryList {
            deliveries: state
                .log
                .get(id)
                .map(|l| l.iter().map(|l| l.view.clone()).collect())
                .unwrap_or_default(),
        })
    }
}

/// The webhooks admin service.
#[derive(Debug)]
pub struct WebhooksService<T> {
    hooks: Arc<Webhooks<T>>,
}

impl<T> WebhooksService<T> {
    /// The service over `hooks`.
    pub const fn new(hooks: Arc<Webhooks<T>>) -> Self {
        Self { hooks }
    }
}

macro_rules! admin {
    ($s:expr, |$h:ident, $ctx:ident, $input:ident : $ty:ty| $body:expr) => {{
        let hooks = Arc::clone(&$s.hooks);
        move |$ctx: RequestContext, $input: $ty| {
            let $h = Arc::clone(&hooks);
            async move {
                // The gateway checks the scope; this keeps share links out
                // even if a route is ever misdeclared.
                $ctx.principal.require_scope(Scope::Admin)?;
                $body
            }
        }
    }};
}

impl<T: HttpTransport + 'static> Service for WebhooksService<T> {
    fn name(&self) -> &'static str {
        "webhooks"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        vec![
            Operation::query(
                "webhooks.list",
                "/v1/admin/webhooks",
                Scope::Admin,
                admin!(self, |h, ctx, _i: NoInput| Ok(h
                    .store()
                    .list(Timestamp::now()))),
            )
            .summary("Every webhook, with its health and last delivery"),
            Operation::mutation(
                "webhooks.create",
                HttpMethod::Post,
                "/v1/admin/webhooks",
                Scope::Admin,
                admin!(self, |h, ctx, input: HookInput| {
                    let by = ctx.principal.user_id().map(|u| u.to_string());
                    h.store().create(&input, by, Timestamp::now())
                }),
            )
            .summary("Add a webhook; its signing secret is shown once"),
            Operation::mutation(
                "webhooks.update",
                HttpMethod::Put,
                "/v1/admin/webhooks/{id}",
                Scope::Admin,
                admin!(self, |h, ctx, input: HookUpdate| h
                    .store()
                    .update(&input, Timestamp::now())),
            )
            .summary("Change a webhook's URL, events, teams or turn it on or off"),
            Operation::mutation(
                "webhooks.delete",
                HttpMethod::Delete,
                "/v1/admin/webhooks/{id}",
                Scope::Admin,
                admin!(self, |h, ctx, input: HookId| {
                    h.store().delete(&input.id)?;
                    Ok(Done { ok: true })
                }),
            )
            .summary("Remove a webhook"),
            Operation::mutation(
                "webhooks.rotate_secret",
                HttpMethod::Post,
                "/v1/admin/webhooks/{id}/secret",
                Scope::Admin,
                admin!(self, |h, ctx, input: HookId| {
                    let secret = h.store().rotate(&input.id, Timestamp::now())?;
                    let hook = h
                        .store()
                        .list(Timestamp::now())
                        .hooks
                        .into_iter()
                        .find(|v| v.id == input.id)
                        .ok_or_else(not_found)?;
                    Ok(HookCreated { hook, secret })
                }),
            )
            .summary("Make a new signing secret; the old one signs for 24 more hours"),
            Operation::mutation(
                "webhooks.test",
                HttpMethod::Post,
                "/v1/admin/webhooks/{id}/test",
                Scope::Admin,
                admin!(self, |h, ctx, input: HookId| h
                    .ping(&input.id, Timestamp::now())
                    .await),
            )
            .summary("Send a signed test delivery now"),
            Operation::query(
                "webhooks.deliveries",
                "/v1/admin/webhooks/{id}/deliveries",
                Scope::Admin,
                admin!(self, |h, ctx, input: HookId| h
                    .store()
                    .deliveries(&input.id)),
            )
            .summary("A webhook's last 100 delivery attempts"),
            Operation::mutation(
                "webhooks.redeliver",
                HttpMethod::Post,
                "/v1/admin/webhooks/{id}/redeliver",
                Scope::Admin,
                admin!(self, |h, ctx, input: RedeliverInput| {
                    h.redeliver(&input.id, &input.delivery_id, Timestamp::now())?;
                    Ok(Done { ok: true })
                }),
            )
            .summary("Send a logged event to a webhook again"),
        ]
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

    use std::sync::Mutex as StdMutex;

    use super::*;
    use crate::delivery::{HttpResponse, TransportError};
    use crate::endpoint::policy;
    use crate::webhook::OutboundRequest;

    struct Receiver {
        status: StdMutex<u16>,
        got: StdMutex<Vec<OutboundRequest>>,
    }

    impl Receiver {
        fn new(status: u16) -> Self {
            Self {
                status: StdMutex::new(status),
                got: StdMutex::new(Vec::new()),
            }
        }
    }

    impl HttpTransport for &Receiver {
        async fn post(&self, request: &OutboundRequest) -> Result<HttpResponse, TransportError> {
            self.got.lock().unwrap().push(request.clone());
            Ok(HttpResponse {
                status: *self.status.lock().unwrap(),
                retry_after: None,
            })
        }
    }

    fn at(minutes: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + minutes * 60).unwrap()
    }

    fn input(url: &str, types: &[&str], teams: &[&str]) -> HookInput {
        HookInput {
            url: url.to_owned(),
            description: "CI bridge".to_owned(),
            event_types: types.iter().map(|t| (*t).to_owned()).collect(),
            teams: teams.iter().map(|t| (*t).to_owned()).collect(),
        }
    }

    fn issue_event(team: &str) -> WebhookEvent {
        WebhookEvent {
            id: format!("ev_{team}"),
            event_type: EventType::Issue,
            action: EventAction::Create,
            occurred_at: at(0),
            team: Some(team.to_owned()),
            actor: None,
            data: json!({"key": format!("{team}-1")}),
            updated_from: None,
            url: Some(format!("/issues/{team}-1")),
        }
    }

    fn hooks(receiver: &Receiver) -> Webhooks<&Receiver> {
        Webhooks::new(Arc::new(HookStore::memory(policy(&[]))), receiver)
    }

    #[test]
    fn urls_are_checked_when_saved() {
        let store = HookStore::memory(policy(&[]));
        for bad in [
            "http://hooks.northstar.dev/x",
            "https://127.0.0.1/x",
            "https://10.0.0.8/x",
            "https://hooks.northstar.dev:22/x",
            "https://user:pw@hooks.northstar.dev/x",
            "file:///etc/passwd",
        ] {
            assert!(
                store
                    .create(&input(bad, &["issue"], &[]), None, at(0))
                    .is_err(),
                "{bad}"
            );
        }
        assert!(
            store
                .create(
                    &input("https://hooks.northstar.dev/x", &["party"], &[]),
                    None,
                    at(0)
                )
                .is_err()
        );
        assert!(
            store
                .create(
                    &input("https://hooks.northstar.dev/x", &[], &[]),
                    None,
                    at(0)
                )
                .is_err()
        );
        assert!(
            store
                .create(
                    &input("https://hooks.northstar.dev/x", &["issue"], &["E N G"]),
                    None,
                    at(0)
                )
                .is_err()
        );
        let ok = store
            .create(
                &input("https://hooks.northstar.dev/x", &["issue"], &["eng"]),
                None,
                at(0),
            )
            .unwrap();
        assert!(ok.secret.starts_with("whsec_"));
        assert_eq!(ok.hook.teams, vec!["ENG"]);
        assert!(ok.secret.ends_with(&ok.hook.secret_hint));
        // The secret is not in the list.
        let listed = serde_json::to_string(&store.list(at(0))).unwrap();
        assert!(!listed.contains(&ok.secret));
    }

    #[tokio::test]
    async fn events_reach_matching_hooks_signed_and_private_teams_stay_private() {
        let receiver = Receiver::new(204);
        let hooks = hooks(&receiver);
        let all = hooks
            .store()
            .create(
                &input("https://a.northstar.dev/h", &["issue"], &[]),
                None,
                at(0),
            )
            .unwrap();
        let _ = hooks
            .store()
            .create(
                &input("https://b.northstar.dev/h", &["comment"], &[]),
                None,
                at(0),
            )
            .unwrap();
        let private = BTreeSet::from(["SEC".to_owned()]);
        let queued = hooks.publish(&[issue_event("ENG"), issue_event("SEC")], &private, at(0));
        assert_eq!(queued, 1, "only the public ENG issue, only the issue hook");
        let report = hooks.run_due(at(0)).await;
        assert_eq!(report.delivered, 1);
        let got = receiver.got.lock().unwrap();
        assert_eq!(got[0].url.as_str(), "https://a.northstar.dev/h");
        let header = got[0].header(crate::SIGNATURE_HEADER).unwrap().to_owned();
        let secret = WebhookSecret::parse(&all.secret).unwrap();
        assert!(
            crate::signature::verify(
                &header,
                &got[0].body,
                &secret,
                at(0),
                SignedDuration::from_mins(5)
            )
            .is_ok()
        );
        drop(got);
        let log = hooks.store().deliveries(&all.hook.id).unwrap().deliveries;
        assert_eq!(log[0].event, "issue.create");
        assert_eq!(log[0].status, Some(204));
    }

    #[tokio::test]
    async fn failures_retry_then_redeliver_and_pause() {
        let receiver = Receiver::new(500);
        let hooks = hooks(&receiver);
        let hook = hooks
            .store()
            .create(
                &input("https://a.northstar.dev/h", &["issue"], &[]),
                None,
                at(0),
            )
            .unwrap()
            .hook;
        let _ = hooks.publish(&[issue_event("ENG")], &BTreeSet::new(), at(0));
        let report = hooks.run_due(at(0)).await;
        assert_eq!(report.retrying, 1);
        assert_eq!(
            hooks.run_due(at(0)).await,
            HookReport::default(),
            "waits for the backoff"
        );
        assert_eq!(
            hooks.run_due(at(1)).await.retrying,
            1,
            "first retry after a minute"
        );

        // Redelivering from the log queues it again at once.
        *receiver.status.lock().unwrap() = 200;
        let log = hooks.store().deliveries(&hook.id).unwrap().deliveries;
        hooks
            .redeliver(&hook.id, &log[0].delivery_id, at(2))
            .unwrap();
        assert_eq!(hooks.run_due(at(2)).await.delivered, 1);

        // A hook failing for days is paused and its queue dropped.
        *receiver.status.lock().unwrap() = 500;
        let mut now = at(10);
        for day in 0..EndpointHealth::PAUSE_AFTER_FAILURES {
            let _ = hooks.publish(&[issue_event("ENG")], &BTreeSet::new(), now);
            // Run past every retry.
            for _ in 0..10 {
                now = now.checked_add(SignedDuration::from_hours(25)).unwrap();
                let _ = hooks.run_due(now).await;
            }
            let _ = day;
        }
        let listed = hooks.store().list(now);
        assert!(!listed.hooks[0].enabled);
        assert!(listed.hooks[0].paused_reason.is_some());
        assert_eq!(hooks.store().pending(), 0);
    }

    #[tokio::test]
    async fn ping_and_rotation_sign_with_both_secrets_during_the_grace() {
        let receiver = Receiver::new(200);
        let hooks = hooks(&receiver);
        let created = hooks
            .store()
            .create(
                &input("https://a.northstar.dev/h", &["comment"], &[]),
                None,
                at(0),
            )
            .unwrap();
        let new = hooks.store().rotate(&created.hook.id, at(0)).unwrap();
        assert_ne!(new, created.secret);
        let ping = hooks.ping(&created.hook.id, at(1)).await.unwrap();
        assert_eq!((ping.event.as_str(), ping.status), ("ping", Some(200)));
        let first = receiver.got.lock().unwrap()[0].clone();
        assert_eq!(first.header(EVENT_HEADER), Some("ping"));
        let header = first.header(crate::SIGNATURE_HEADER).unwrap();
        assert_eq!(
            header.matches("v1=").count(),
            2,
            "old and new secret both sign"
        );
        let after = hooks.store().list(at(24 * 60 + 2)).hooks;
        assert!(!after[0].rotating);
        assert!(hooks.ping("wh_nope", at(1)).await.is_err());
    }

    #[test]
    fn state_survives_a_restart_and_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = HookStore::open(dir.path(), policy(&[])).unwrap();
        let created = store
            .create(
                &input("https://a.northstar.dev/h", &["issue"], &[]),
                None,
                at(0),
            )
            .unwrap();
        let again = HookStore::open(dir.path(), policy(&[])).unwrap();
        assert_eq!(again.list(at(0)).hooks[0].id, created.hook.id);
        again.delete(&created.hook.id).unwrap();
        assert!(again.delete(&created.hook.id).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.path().join(".vellum/webhooks.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
    }
}
