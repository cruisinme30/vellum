// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Connecting Google Calendar and running the time-block sync (R-MW-9).
//!
//! The person connects once: [`TasksService::google_connect`] returns
//! Google's consent page, and Google sends them back to [`CALLBACK`] with a
//! code. Vellum asks only for the `calendar.app.created` scope: it can
//! create its own calendar and manage the events in it, and cannot see
//! the person's other calendars. It keeps the refresh token through
//! [`GoogleLinks`], creates the "Vellum" calendar and syncs.
//!
//! After that, [`TasksService::google_sync`] runs on a timer in the
//! server and on demand. It pulls the calendar's changes with Google's
//! incremental sync token, plans with [`crate::timeblock::plan`] and
//! carries the steps out.
//!
//! The Google API itself is behind [`GoogleCalendar`], which the binary
//! implements over HTTPS.

use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::sync::{Arc, Mutex};

use jiff::{SignedDuration, Timestamp};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use vellum_contracts::BoxFuture;
use vellum_kernel::{BlockId, Error, ErrorKind, ShareToken, UserId};

use crate::checklist::{self, Checklist};
use crate::model::{Source, TaskFields};
use crate::raw::{RawRequest, RawResponse};
use crate::service::{Done, NoInput, SecretView, TasksService, edit_error, invalid, parse_task_id};
use crate::timeblock::{self, Blocked, CALENDAR_NAME, Remote, Step, SyncState, TimeBlock};

/// Where Google sends the person back after consent.
pub const CALLBACK: &str = "/connect/google/callback";

/// The one scope Vellum asks for.
pub const SCOPE: &str = "https://www.googleapis.com/auth/calendar.app.created";

/// Google's consent page.
pub const AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";

/// How long a consent link stays usable.
const PENDING_TTL: SignedDuration = SignedDuration::from_mins(10);

/// The most consent links waiting at once; older ones are dropped.
const MAX_PENDING: usize = 1024;

/// A person's connection to Google Calendar.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleLink {
    /// The `OAuth` refresh token. Never logged.
    pub refresh_token: String,
    /// The ID of the "Vellum" calendar.
    pub calendar: String,
    /// Where the sync left off.
    #[serde(default)]
    pub state: SyncState,
}

impl fmt::Debug for GoogleLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleLink")
            .field("calendar", &self.calendar)
            .finish_non_exhaustive()
    }
}

/// A page of calendar changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Changes {
    /// The events that changed, and the token for next time.
    Page {
        /// The events, as the Calendar API returns them.
        events: Vec<Value>,
        /// The next sync token.
        next_sync_token: Option<String>,
    },
    /// The sync token expired; a full sync is needed.
    Expired,
}

/// Google Calendar, as the sync needs it. Every call names the person's
/// refresh token; the implementation trades it for access tokens.
pub trait GoogleCalendar: Send + Sync + 'static {
    /// Trades an authorization code for a refresh token.
    fn exchange(
        &self,
        code: String,
        verifier: String,
        redirect: String,
    ) -> BoxFuture<'_, Result<String, Error>>;

    /// Creates a calendar and returns its ID.
    fn create_calendar(
        &self,
        grant: String,
        name: String,
        zone: String,
    ) -> BoxFuture<'_, Result<String, Error>>;

    /// The events changed since `sync_token`, or all of them without one,
    /// deleted ones included.
    fn changes(
        &self,
        grant: String,
        calendar: String,
        sync_token: Option<String>,
    ) -> BoxFuture<'_, Result<Changes, Error>>;

    /// Creates an event and returns its ID.
    fn insert(
        &self,
        grant: String,
        calendar: String,
        body: Value,
    ) -> BoxFuture<'_, Result<String, Error>>;

    /// Updates an event. A missing event fails with `not_found`.
    fn patch(
        &self,
        grant: String,
        calendar: String,
        event: String,
        body: Value,
    ) -> BoxFuture<'_, Result<(), Error>>;

    /// Deletes an event; one already gone is not an error.
    fn delete(
        &self,
        grant: String,
        calendar: String,
        event: String,
    ) -> BoxFuture<'_, Result<(), Error>>;

    /// Revokes the grant.
    fn revoke(&self, grant: String) -> BoxFuture<'_, Result<(), Error>>;
}

/// Where connections are kept.
pub trait GoogleLinks: Send + Sync + 'static {
    /// A person's connection.
    fn link(&self, user: UserId) -> BoxFuture<'_, Result<Option<GoogleLink>, Error>>;

    /// Stores (or with `None`, removes) a person's connection.
    fn set_link(&self, user: UserId, link: Option<GoogleLink>) -> BoxFuture<'_, Result<(), Error>>;

    /// Everyone connected, for the timer.
    fn linked(&self) -> BoxFuture<'_, Result<Vec<UserId>, Error>>;
}

#[derive(Debug, Clone)]
struct Pending {
    user: UserId,
    verifier: String,
    expires: Timestamp,
}

/// Google Calendar, once the server has an `OAuth` client for it.
pub struct Google {
    api: Arc<dyn GoogleCalendar>,
    links: Arc<dyn GoogleLinks>,
    client_id: String,
    pending: Mutex<HashMap<String, Pending>>,
}

impl fmt::Debug for Google {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Google").finish_non_exhaustive()
    }
}

impl Google {
    /// Google Calendar through `api`, with connections kept in `links`,
    /// for the `OAuth` client `client_id`.
    #[must_use]
    pub fn new(
        api: Arc<dyn GoogleCalendar>,
        links: Arc<dyn GoogleLinks>,
        client_id: impl Into<String>,
    ) -> Self {
        Self {
            api,
            links,
            client_id: client_id.into(),
            pending: Mutex::new(HashMap::new()),
        }
    }

    fn remember(&self, state: String, pending: Pending, now: Timestamp) -> Result<(), Error> {
        let poisoned = || Error::new(ErrorKind::Internal, "consent links lock poisoned");
        let mut map = self.pending.lock().map_err(|_| poisoned())?;
        map.retain(|_, p| p.expires > now);
        if map.len() >= MAX_PENDING
            && let Some(oldest) = map
                .iter()
                .min_by_key(|(_, p)| p.expires)
                .map(|(k, _)| k.clone())
        {
            let _ = map.remove(&oldest);
        }
        let _ = map.insert(state, pending);
        drop(map);
        Ok(())
    }

    fn take(&self, state: &str, now: Timestamp) -> Option<Pending> {
        let mut map = self.pending.lock().ok()?;
        map.remove(state).filter(|p| p.expires > now)
    }
}

/// Whether Google Calendar is set up and connected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GoogleView {
    /// Whether the server has a Google `OAuth` client.
    pub configured: bool,
    /// Whether the person has connected their calendar.
    pub connected: bool,
}

/// What a sync did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SyncView {
    /// Events created for tasks.
    pub created: usize,
    /// Events updated from tasks.
    pub updated: usize,
    /// Events deleted.
    pub deleted: usize,
    /// Tasks moved because their event moved.
    pub moved: usize,
    /// Tasks whose event was deleted, so they lost their block.
    pub unblocked: usize,
    /// Tasks added from new events.
    pub adopted: usize,
}

fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let chars = chunk.len() + 1;
        for i in 0..chars {
            let index = (n >> (18 - 6 * i)) & 63;
            if let Some(&c) = ALPHABET.get(index as usize) {
                out.push(char::from(c));
            }
        }
    }
    out
}

/// Percent-encodes a query value.
fn encode(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

/// Decodes one percent-encoded query value (`+` is a space).
fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&byte) = bytes.get(i) {
        match byte {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = bytes
                    .get(i + 1..i + 3)
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                if let Some(decoded) = hex {
                    out.push(decoded);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            other => out.push(other),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then(|| decode(value))
    })
}

fn random() -> Result<String, Error> {
    let token = |_| {
        ShareToken::generate()
            .map(|t| t.expose().to_owned())
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))
    };
    // 66 characters of base62: a PKCE verifier needs 43 to 128.
    (0..3).map(token).collect()
}

impl TasksService {
    /// Turns on Google Calendar time-blocking.
    #[must_use]
    pub fn with_google(mut self, google: Google) -> Self {
        self.google = Some(Arc::new(google));
        self
    }

    fn google(&self) -> Result<&Arc<Google>, Error> {
        self.google.as_ref().ok_or_else(|| {
            Error::new(
                ErrorKind::Unavailable,
                "Google Calendar is not set up on this server",
            )
        })
    }

    fn redirect_uri(&self) -> String {
        format!("{}{CALLBACK}", self.config.base_url)
    }

    /// Whether Google Calendar is set up and connected.
    ///
    /// # Errors
    ///
    /// Fails when the connection store does.
    pub async fn google_status(&self, user: UserId, _input: NoInput) -> Result<GoogleView, Error> {
        let Some(google) = &self.google else {
            return Ok(GoogleView {
                configured: false,
                connected: false,
            });
        };
        Ok(GoogleView {
            configured: true,
            connected: google.links.link(user).await?.is_some(),
        })
    }

    /// Starts connecting Google Calendar: returns the consent page URL.
    ///
    /// # Errors
    ///
    /// `unavailable` when the server has no Google client.
    #[expect(clippy::unused_async, reason = "every operation handler is async")]
    pub async fn google_connect(&self, user: UserId, _input: NoInput) -> Result<SecretView, Error> {
        let google = self.google()?;
        let state = random()?;
        let verifier = random()?;
        let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
        let now = self.now();
        google.remember(
            state.clone(),
            Pending {
                user,
                verifier,
                expires: now.checked_add(PENDING_TTL).unwrap_or(now),
            },
            now,
        )?;
        let params = [
            ("client_id", google.client_id.as_str()),
            ("redirect_uri", &self.redirect_uri()),
            ("response_type", "code"),
            ("scope", SCOPE),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("state", &state),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
        ];
        let query: Vec<String> = params
            .iter()
            .map(|(k, v)| format!("{k}={}", encode(v)))
            .collect();
        Ok(SecretView {
            value: format!("{AUTHORIZE_URL}?{}", query.join("&")),
        })
    }

    /// Disconnects Google Calendar: revokes the grant and unlinks the
    /// tasks. Their blocks stay; the events stay in Google until the
    /// person removes the "Vellum" calendar.
    ///
    /// # Errors
    ///
    /// `unavailable` when the server has no Google client.
    pub async fn google_disconnect(&self, user: UserId, _input: NoInput) -> Result<Done, Error> {
        let google = self.google()?;
        let Some(link) = google.links.link(user).await? else {
            return Ok(Done {});
        };
        if let Err(error) = google.api.revoke(link.refresh_token.clone()).await {
            tracing::warn!(%error, "Google grant not revoked");
        }
        google.links.set_link(user, None).await?;
        let me = self.me(user).await?;
        for task in self.gather(user, &me.profile.handle).await? {
            if task.fields.event.is_some() && matches!(task.source, Source::Checklist { .. }) {
                self.set_fields(user, &task.source.key(), |f| f.event = None)
                    .await?;
            }
        }
        Ok(Done {})
    }

    async fn set_fields(
        &self,
        user: UserId,
        task: &str,
        change: impl Fn(&mut TaskFields) + Send + Sync,
    ) -> Result<(), Error> {
        let (doc, block) = parse_task_id(task)?;
        let _ = self
            .edit(user, doc, |file| {
                let list = Checklist::parse(&file.text);
                let item = list
                    .find(block)
                    .ok_or_else(|| Error::new(ErrorKind::NotFound, "no such task"))?;
                let mut fields = item.fields.clone();
                change(&mut fields);
                checklist::update(&file.text, block, &item.title, &fields)
                    .map(|text| (text, ()))
                    .map_err(edit_error)
            })
            .await?;
        Ok(())
    }

    /// Syncs time blocks with the person's "Vellum" calendar, both ways.
    ///
    /// # Errors
    ///
    /// `unavailable` when the server has no Google client,
    /// `invalid_input` when the person has not connected, or a Google or
    /// workspace failure. Progress made before a failure is kept.
    pub async fn google_sync(&self, user: UserId, _input: NoInput) -> Result<SyncView, Error> {
        let google = self.google()?;
        let mut link = google
            .links
            .link(user)
            .await?
            .ok_or_else(|| invalid("connect Google Calendar first"))?;
        let result = self.sync_link(google, user, &mut link).await;
        google.links.set_link(user, Some(link)).await?;
        result
    }

    async fn sync_link(
        &self,
        google: &Google,
        user: UserId,
        link: &mut GoogleLink,
    ) -> Result<SyncView, Error> {
        let me = self.me(user).await?;
        let zone = me.profile.time_zone.clone();
        let zone_name = zone.iana_name().unwrap_or("UTC").to_owned();
        let tasks: Vec<Blocked> = self
            .gather(user, &me.profile.handle)
            .await?
            .into_iter()
            .filter(|t| matches!(t.source, Source::Checklist { .. }))
            .filter(|t| t.fields.block.is_some() || t.fields.event.is_some())
            .map(|t| Blocked {
                task: t.source.key(),
                title: t.title,
                block: t.fields.block,
                done: t.done,
                event: t.fields.event,
            })
            .collect();
        let grant = link.refresh_token.clone();
        let calendar = link.calendar.clone();
        let mut changes = google
            .api
            .changes(
                grant.clone(),
                calendar.clone(),
                link.state.sync_token.clone(),
            )
            .await?;
        if changes == Changes::Expired {
            link.state.sync_token = None;
            changes = google
                .api
                .changes(grant.clone(), calendar.clone(), None)
                .await?;
        }
        let Changes::Page {
            events,
            next_sync_token,
        } = changes
        else {
            return Err(Error::new(
                ErrorKind::Unavailable,
                "Google Calendar refused a full sync",
            ));
        };
        let remote: Vec<Remote> = events
            .iter()
            .filter_map(|e| timeblock::parse_event(e, &zone))
            .collect();
        let steps = timeblock::plan(&tasks, &remote, &link.state);
        let mut view = SyncView::default();
        for step in steps {
            self.run_step(google, user, link, &zone_name, step, &mut view)
                .await?;
        }
        link.state.sync_token = next_sync_token;
        tracing::info!(?view, "Google Calendar synced");
        Ok(view)
    }

    fn task_link(&self, task: &str) -> String {
        format!("{}/my-work?task={task}", self.config.base_url)
    }

    async fn run_step(
        &self,
        google: &Google,
        user: UserId,
        link: &mut GoogleLink,
        zone: &str,
        step: Step,
        view: &mut SyncView,
    ) -> Result<(), Error> {
        let grant = link.refresh_token.clone();
        let calendar = link.calendar.clone();
        match step {
            Step::Move { task, event, block } => {
                self.set_fields(user, &task, |f| {
                    f.block = Some(block);
                    f.event = Some(event.clone());
                })
                .await?;
                view.moved += 1;
            }
            Step::Unblock { task, event } => {
                self.set_fields(user, &task, |f| {
                    f.block = None;
                    f.event = None;
                })
                .await?;
                if let Some(event) = event {
                    let _ = link.state.pushed.remove(&event);
                }
                view.unblocked += 1;
            }
            Step::Adopt {
                event,
                title,
                block,
            } => {
                let task = self.adopt(user, &event, &title, block).await?;
                let body = timeblock::event_body(&task, block, zone, &self.task_link(&task.task));
                google
                    .api
                    .patch(grant, calendar, event.clone(), body)
                    .await?;
                let _ = link.state.pushed.insert(event, task.fingerprint());
                view.adopted += 1;
            }
            Step::Insert { task } => {
                let Some(block) = task.block else {
                    return Ok(());
                };
                let body = timeblock::event_body(&task, block, zone, &self.task_link(&task.task));
                let event = google.api.insert(grant, calendar, body).await?;
                let _ = link.state.pushed.insert(event.clone(), task.fingerprint());
                self.set_fields(user, &task.task, |f| f.event = Some(event.clone()))
                    .await?;
                view.created += 1;
            }
            Step::Patch { event, task } => {
                let Some(block) = task.block else {
                    return Ok(());
                };
                let body = timeblock::event_body(&task, block, zone, &self.task_link(&task.task));
                match google.api.patch(grant, calendar, event.clone(), body).await {
                    Ok(()) => {
                        let _ = link.state.pushed.insert(event, task.fingerprint());
                        view.updated += 1;
                    }
                    Err(err) if err.kind() == ErrorKind::NotFound => {
                        // Gone from the calendar: unlink, and the next
                        // sync creates a fresh event.
                        let _ = link.state.pushed.remove(&event);
                        self.set_fields(user, &task.task, |f| f.event = None)
                            .await?;
                    }
                    Err(err) => return Err(err),
                }
            }
            Step::Delete { event } => {
                google.api.delete(grant, calendar, event.clone()).await?;
                let _ = link.state.pushed.remove(&event);
                view.deleted += 1;
            }
        }
        Ok(())
    }

    /// Adds a task to the Inbox for a new calendar event.
    async fn adopt(
        &self,
        user: UserId,
        event: &str,
        title: &str,
        block: TimeBlock,
    ) -> Result<Blocked, Error> {
        let me = self.me(user).await?;
        let doc = self.inbox(user, &me.profile.handle).await?;
        let title = if title.trim().is_empty() {
            "Time block"
        } else {
            title.trim()
        };
        let fields = TaskFields {
            block: Some(block),
            event: Some(event.to_owned()),
            src: Some(format!("gcal:{event}")),
            ..TaskFields::default()
        };
        let id = BlockId::generate();
        let _ = self
            .edit(user, doc, |file| {
                checklist::append(&file.text, None, title, &fields, id)
                    .map(|text| (text, ()))
                    .map_err(edit_error)
            })
            .await?;
        Ok(Blocked {
            task: format!("{doc}.{id}"),
            title: title.to_owned(),
            block: Some(block),
            done: false,
            event: Some(event.to_owned()),
        })
    }

    /// The consent callback: stores the connection, creates the calendar
    /// and runs the first sync, then sends the person back to My Work.
    pub(crate) async fn google_callback(&self, request: &RawRequest) -> Result<RawResponse, Error> {
        let back = |outcome: &str| {
            RawResponse::redirect(&format!(
                "{}/my-work?google={outcome}",
                self.config.base_url
            ))
        };
        if request.method != "GET" {
            return Ok(RawResponse::text(405, "method not allowed").with_header("Allow", "GET"));
        }
        let google = self.google()?;
        let state = query_param(&request.query, "state").unwrap_or_default();
        let Some(pending) = google.take(&state, self.now()) else {
            return Ok(back("expired"));
        };
        let Some(code) = query_param(&request.query, "code") else {
            return Ok(back("declined"));
        };
        let user = pending.user;
        let refresh_token = google
            .api
            .exchange(code, pending.verifier, self.redirect_uri())
            .await?;
        let me = self.me(user).await?;
        let zone = me.profile.time_zone.iana_name().unwrap_or("UTC").to_owned();
        let calendar = match google.links.link(user).await? {
            Some(old) => old.calendar,
            None => {
                google
                    .api
                    .create_calendar(refresh_token.clone(), CALENDAR_NAME.to_owned(), zone)
                    .await?
            }
        };
        google
            .links
            .set_link(
                user,
                Some(GoogleLink {
                    refresh_token,
                    calendar,
                    state: SyncState::default(),
                }),
            )
            .await?;
        if let Err(error) = self.google_sync(user, NoInput {}).await {
            tracing::warn!(%error, "first Google Calendar sync failed");
        }
        Ok(back("connected"))
    }

    /// Syncs everyone who connected Google Calendar; the server calls it
    /// on a timer. Failures are logged per person.
    ///
    /// # Errors
    ///
    /// Fails only when the connection list cannot be read.
    pub async fn google_sync_all(&self) -> Result<usize, Error> {
        let Some(google) = &self.google else {
            return Ok(0);
        };
        let mut synced = 0;
        for user in google.links.linked().await? {
            match self.google_sync(user, NoInput {}).await {
                Ok(_) => synced += 1,
                Err(error) => tracing::warn!(%error, %user, "Google Calendar sync failed"),
            }
        }
        Ok(synced)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_matches_the_pkce_example() {
        // RFC 7636, appendix B.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            base64url(&Sha256::digest(verifier.as_bytes())),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn query_values_round_trip() {
        let value = "a b&c=d/é";
        let query = format!("x=1&state={}", encode(value));
        assert_eq!(query_param(&query, "state").as_deref(), Some(value));
        assert_eq!(query_param(&query, "missing"), None);
    }

    #[test]
    fn verifiers_are_long_enough() {
        let v = random().unwrap();
        assert!((43..=128).contains(&v.len()));
    }
}
