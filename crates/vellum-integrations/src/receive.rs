// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! HTTP receivers for providers that call Vellum: GitHub, GitLab, Slack,
//! Sentry and the inbound mail adapter.
//!
//! | Route | Authenticated by | Does |
//! |---|---|---|
//! | `POST /github` | `X-Hub-Signature-256` | Git automation |
//! | `POST /gitlab` | `X-Gitlab-Token` | Git automation |
//! | `POST /slack` | `X-Slack-Signature` (5-minute window) | files an issue from a command or message |
//! | `POST /sentry` | `Sentry-Hook-Signature` | files an issue from an alert |
//! | `POST /email` | `Authorization: Bearer <mail token>` | files an issue, or adds a reply as a comment |
//!
//! The router is mounted cross-site (providers do not send browser CSRF
//! tokens), so each route checks its own signature over the raw body before
//! anything is parsed. A provider with no secret configured is refused with
//! `404`: no secret never means "accept anything". Bad signatures get
//! `401`, malformed bodies `400`, and a failure in the issue core `503`, so
//! the provider retries; retries are safe because intake dedupes by source
//! and Git automation is idempotent.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use jiff::Timestamp;
use serde_json::json;

use crate::git::service::{LinkStore, Settings, apply};
use crate::git::{github, gitlab};
use crate::inbound::InboundError;
use crate::intake::Routing;
use crate::intake::email::{self, EmailIntake, EmailSettings};
use crate::intake::{sentry, slack};
use crate::mac;
use crate::model::{WorkCommands, WorkError};

/// Secrets and settings of the receivers. An empty secret turns its route
/// off.
#[derive(Clone)]
pub struct ReceiverConfig {
    /// The GitHub webhook secret.
    pub github_secret: Vec<u8>,
    /// The GitLab webhook token.
    pub gitlab_token: String,
    /// The Slack app's signing secret.
    pub slack_signing_secret: Vec<u8>,
    /// The Sentry integration's client secret.
    pub sentry_client_secret: Vec<u8>,
    /// The token the inbound mail adapter sends as a bearer token.
    pub mail_token: Vec<u8>,
    /// Connected repositories and each team's state mapping.
    pub git: Settings,
    /// Where Slack and Sentry intake lands.
    pub routing: Routing,
    /// Sentry project slug to team key.
    pub sentry_projects: BTreeMap<String, String>,
    /// Intake and reply addresses.
    pub email: EmailSettings,
}

impl std::fmt::Debug for ReceiverConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Secrets never reach logs; show only which routes are on.
        f.debug_struct("ReceiverConfig")
            .field("github", &!self.github_secret.is_empty())
            .field("gitlab", &!self.gitlab_token.is_empty())
            .field("slack", &!self.slack_signing_secret.is_empty())
            .field("sentry", &!self.sentry_client_secret.is_empty())
            .field("email", &!self.mail_token.is_empty())
            .finish_non_exhaustive()
    }
}

/// The receivers' state.
pub struct Receivers<W, L> {
    work: W,
    links: L,
    config: ReceiverConfig,
    clock: fn() -> Timestamp,
}

impl<W, L> std::fmt::Debug for Receivers<W, L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Secrets stay out of logs.
        f.debug_struct("Receivers").finish_non_exhaustive()
    }
}

impl<W: WorkCommands + 'static, L: LinkStore + 'static> Receivers<W, L> {
    /// Receivers acting through `work` and `links`.
    #[must_use]
    pub fn new(work: W, links: L, config: ReceiverConfig) -> Self {
        Self {
            work,
            links,
            config,
            clock: Timestamp::now,
        }
    }

    /// Uses `clock` instead of the system time (for tests).
    #[must_use]
    pub fn with_clock(mut self, clock: fn() -> Timestamp) -> Self {
        self.clock = clock;
        self
    }

    /// The routes, to mount cross-site under a prefix such as `/hooks`.
    pub fn router(self: Arc<Self>) -> Router {
        Router::new()
            .route("/github", post(github_hook::<W, L>))
            .route("/gitlab", post(gitlab_hook::<W, L>))
            .route("/slack", post(slack_hook::<W, L>))
            .route("/sentry", post(sentry_hook::<W, L>))
            .route("/email", post(email_hook::<W, L>))
            .with_state(self)
    }
}

/// Why a delivery was not accepted.
#[derive(Debug)]
enum Refusal {
    NotConfigured,
    Inbound(InboundError),
    Work(WorkError),
}

impl From<InboundError> for Refusal {
    fn from(e: InboundError) -> Self {
        Self::Inbound(e)
    }
}

impl From<WorkError> for Refusal {
    fn from(e: WorkError) -> Self {
        Self::Work(e)
    }
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            Self::NotConfigured => (
                StatusCode::NOT_FOUND,
                "integration not configured".to_owned(),
            ),
            Self::Inbound(e @ InboundError::Unauthenticated) => {
                (StatusCode::UNAUTHORIZED, e.to_string())
            }
            Self::Inbound(e @ InboundError::Malformed(_)) => {
                (StatusCode::BAD_REQUEST, e.to_string())
            }
            // The provider retries; intake and automation are idempotent.
            Self::Work(e @ WorkError::Unavailable(_)) => {
                tracing::warn!(error = %e, "integration delivery failed");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "try again later".to_owned(),
                )
            }
            // A missing issue or a refused change will not succeed on retry.
            Self::Work(e) => (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()),
        };
        (status, axum::Json(json!({ "error": message }))).into_response()
    }
}

fn header<'h>(headers: &'h HeaderMap, name: &str) -> Option<&'h str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn accepted(body: serde_json::Value) -> Response {
    (StatusCode::ACCEPTED, axum::Json(body)).into_response()
}

#[tracing::instrument(name = "integrations.receive.github", skip_all)]
async fn github_hook<W: WorkCommands, L: LinkStore>(
    State(r): State<Arc<Receivers<W, L>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Refusal> {
    if r.config.github_secret.is_empty() {
        return Err(Refusal::NotConfigured);
    }
    github::verify(
        &r.config.github_secret,
        header(&headers, "X-Hub-Signature-256"),
        &body,
    )?;
    let event = header(&headers, "X-GitHub-Event").unwrap_or_default();
    if event == "ping" {
        return Ok(accepted(json!({ "ok": true })));
    }
    let Some(event) = github::parse(event, &body)? else {
        return Ok(accepted(json!({ "applied": 0 })));
    };
    let applied = apply(&r.work, &r.links, &r.config.git, &event).await?;
    Ok(accepted(json!({ "applied": applied.len() })))
}

#[tracing::instrument(name = "integrations.receive.gitlab", skip_all)]
async fn gitlab_hook<W: WorkCommands, L: LinkStore>(
    State(r): State<Arc<Receivers<W, L>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Refusal> {
    if r.config.gitlab_token.is_empty() {
        return Err(Refusal::NotConfigured);
    }
    gitlab::verify(&r.config.gitlab_token, header(&headers, "X-Gitlab-Token"))?;
    let event = header(&headers, "X-Gitlab-Event").unwrap_or_default();
    let Some(event) = gitlab::parse(event, &body)? else {
        return Ok(accepted(json!({ "applied": 0 })));
    };
    let applied = apply(&r.work, &r.links, &r.config.git, &event).await?;
    Ok(accepted(json!({ "applied": applied.len() })))
}

/// Slack shows the JSON `text` to the user who ran the command, so every
/// answer, including refusals the user can fix, is a 200 with a message.
#[tracing::instrument(name = "integrations.receive.slack", skip_all)]
async fn slack_hook<W: WorkCommands, L: LinkStore>(
    State(r): State<Arc<Receivers<W, L>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Refusal> {
    if r.config.slack_signing_secret.is_empty() {
        return Err(Refusal::NotConfigured);
    }
    slack::verify(
        &r.config.slack_signing_secret,
        header(&headers, "X-Slack-Request-Timestamp"),
        header(&headers, "X-Slack-Signature"),
        &body,
        (r.clock)(),
    )?;
    let request = slack::parse(&body)?;
    let reply = |text: String| {
        (
            StatusCode::OK,
            axum::Json(json!({ "response_type": "ephemeral", "text": text })),
        )
            .into_response()
    };
    let Some(draft) = slack::to_draft(&request, &r.config.routing, None) else {
        return Ok(reply("Usage: /vellum [TEAM] title of the issue".to_owned()));
    };
    let key = r.work.create_issue(draft).await?;
    Ok(reply(format!("Filed {key} in triage.")))
}

#[tracing::instrument(name = "integrations.receive.sentry", skip_all)]
async fn sentry_hook<W: WorkCommands, L: LinkStore>(
    State(r): State<Arc<Receivers<W, L>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Refusal> {
    if r.config.sentry_client_secret.is_empty() {
        return Err(Refusal::NotConfigured);
    }
    sentry::verify(
        &r.config.sentry_client_secret,
        header(&headers, sentry::SIGNATURE_HEADER),
        &body,
    )?;
    let resource = header(&headers, sentry::RESOURCE_HEADER).unwrap_or_default();
    let Some(draft) = sentry::to_draft(
        resource,
        &body,
        &r.config.routing,
        &r.config.sentry_projects,
    )?
    else {
        return Ok(accepted(json!({ "issue": null })));
    };
    let key = r.work.create_issue(draft).await?;
    Ok(accepted(json!({ "issue": key.to_string() })))
}

#[tracing::instrument(name = "integrations.receive.email", skip_all)]
async fn email_hook<W: WorkCommands, L: LinkStore>(
    State(r): State<Arc<Receivers<W, L>>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Refusal> {
    if r.config.mail_token.is_empty() {
        return Err(Refusal::NotConfigured);
    }
    let given = header(&headers, "Authorization")
        .and_then(|h| h.strip_prefix("Bearer "))
        .unwrap_or_default();
    if !mac::equal(given.trim().as_bytes(), &r.config.mail_token) {
        return Err(InboundError::Unauthenticated.into());
    }
    let mail = email::parse(&body)?;
    match email::classify(&mail, &r.config.email) {
        EmailIntake::NewIssue(draft) => {
            let key = r.work.create_issue(*draft).await?;
            Ok(accepted(json!({ "issue": key.to_string() })))
        }
        EmailIntake::Comment { key, user, body } => {
            r.work.add_comment(&key, Some(&user), &body).await?;
            Ok(accepted(json!({ "comment": key.to_string() })))
        }
        // Dropped mail is still a delivery: a retry would be dropped too.
        EmailIntake::Ignored(reason) => Ok(accepted(json!({ "ignored": reason }))),
    }
}
