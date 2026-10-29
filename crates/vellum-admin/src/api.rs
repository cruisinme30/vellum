// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The admin service's operations on the gateway seam
//! ([`vellum_contracts::Service`]).
//!
//! Every operation needs the `admin` scope except the browser's telemetry
//! report, which any signed-in reader sends. Operations that change state
//! or export the audit log also need a recent sign-in. The gateway records
//! each request into the trace buffer and route metrics before the
//! handler runs, so the handlers only read them.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use vellum_contracts::{
    CredentialKind, HttpMethod, Operation, Principal, RequestContext, Scope, Service,
};
use vellum_crypto::KeyProvider;
use vellum_kernel::{Error, ErrorKind, Result};
use vellum_observe::{TraceId, TraceParent, TraceQuery};

use crate::audit::{AuditActor, AuditQuery, AuditRecord};
use crate::backup::BackupFile;
use crate::service::{AdminService, Caller};
use crate::settings::AdminSettings;
use crate::usage::UsageFacts;

/// How recent a sign-in admin changes need. It matches the default of
/// `security.fresh_auth_minutes`; operations are declared once at start-up.
pub const FRESH_AUTH: Duration = Duration::from_mins(5);

/// Supplies the counts that anonymous telemetry reports (users, spaces,
/// pages), which other services own.
#[async_trait]
pub trait UsageFactsSource: Send + Sync {
    /// The counts now.
    async fn facts(&self) -> UsageFacts;
}

/// A source that reports nothing, until the services that own the counts
/// provide one.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoFacts;

#[async_trait]
impl UsageFactsSource for NoFacts {
    async fn facts(&self) -> UsageFacts {
        UsageFacts::default()
    }
}

/// The admin service as the gateway sees it.
pub struct AdminApi<K: KeyProvider + 'static> {
    service: Arc<AdminService<K>>,
    facts: Arc<dyn UsageFactsSource>,
}

impl<K: KeyProvider + 'static> std::fmt::Debug for AdminApi<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminApi").finish_non_exhaustive()
    }
}

impl<K: KeyProvider + 'static> AdminApi<K> {
    /// Wraps the service.
    #[must_use]
    pub fn new(service: Arc<AdminService<K>>, facts: Arc<dyn UsageFactsSource>) -> Self {
        Self { service, facts }
    }
}

/// The audit actor for a request's principal.
#[must_use]
pub fn caller(ctx: &RequestContext) -> Caller {
    let p: &Principal = &ctx.principal;
    let kind = match p.kind() {
        CredentialKind::Session => "user",
        CredentialKind::ApiToken => "api_token",
        CredentialKind::BreakGlass => "break_glass",
        CredentialKind::Anonymous | CredentialKind::ShareLink => "external",
    };
    let via = match p.kind() {
        CredentialKind::Session => "session",
        CredentialKind::ApiToken => "api_token",
        CredentialKind::BreakGlass => "break_glass",
        CredentialKind::ShareLink => "share_link",
        CredentialKind::Anonymous => "anonymous",
    };
    Caller {
        actor: AuditActor {
            kind: kind.to_owned(),
            user_id: p.user_id().map(|u| u.to_string()),
            via: Some(via.to_owned()),
        },
        request_id: Some(ctx.request_id.to_string()),
        trace_id: ctx
            .traceparent
            .as_deref()
            .and_then(|t| t.parse::<TraceParent>().ok())
            .map(|t| t.trace_id.to_string()),
    }
}

/// No input.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
pub struct Empty {}

/// Filters for the request list.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RequestsInput {
    /// Only this route template.
    pub route: Option<String>,
    /// Only requests at least this long, in milliseconds.
    pub min_duration_ms: Option<u64>,
    /// Only failed requests.
    pub errors_only: bool,
    /// A request id or trace id.
    pub id: Option<String>,
    /// Only requests older than this cursor.
    pub before: Option<u64>,
    /// At most this many rows (up to 500).
    pub limit: Option<usize>,
}

/// The request list, newest first.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RequestsOutput {
    /// One summary per request.
    #[schemars(with = "Vec<Value>")]
    pub requests: Vec<vellum_observe::TraceSummary>,
}

/// One trace.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TraceInput {
    /// The 32-hex-digit W3C trace id.
    pub trace_id: String,
}

/// A window in minutes.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct WindowInput {
    /// Minutes, 1 to 60 (default 15).
    pub minutes: Option<u64>,
}

/// The SLOs.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SlosOutput {
    /// Each SLO with its burn rates and remaining budget.
    #[schemars(with = "Vec<Value>")]
    pub slos: Vec<vellum_observe::slo::SloStatus>,
}

/// What to profile.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ProfileInput {
    /// How far back to look, in seconds (default 60).
    pub seconds: Option<u64>,
    /// Only this route template.
    pub route: Option<String>,
}

/// What the browser reported, joined to its server trace.
#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
pub struct IngestedOutput {
    /// Spans added.
    pub spans: usize,
    /// Whether the server trace was still in the buffer.
    pub joined: bool,
}

/// The audit log as JSON Lines.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AuditExport {
    /// One record per line, oldest first.
    pub jsonl: String,
}

/// Audit records, newest first.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AuditList {
    /// The records.
    pub records: Vec<AuditRecord>,
}

/// New settings and the version they replace.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SettingsUpdate {
    /// The version read; the save fails if someone saved since.
    pub expected_version: u64,
    /// The settings.
    pub settings: AdminSettings,
}

/// The backups on disk, newest first.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct BackupList {
    /// The backups.
    pub backups: Vec<BackupFile>,
}

fn require_admin_user(ctx: &RequestContext) -> Result<()> {
    // The gateway checks the scope; this keeps share links out even if a
    // route is ever misdeclared.
    ctx.principal.require_scope(Scope::Admin)
}

type Svc<K> = Arc<AdminService<K>>;

/// An admin-scope query.
fn query<K, I, O, F, Fut>(
    svc: &Svc<K>,
    id: &'static str,
    path: &'static str,
    summary: &'static str,
    f: F,
) -> Operation
where
    K: KeyProvider + 'static,
    I: serde::de::DeserializeOwned + JsonSchema + Send + 'static,
    O: Serialize + JsonSchema + Send + 'static,
    F: Fn(Svc<K>, RequestContext, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O>> + Send + 'static,
{
    let s = Arc::clone(svc);
    Operation::query(
        id,
        path,
        Scope::Admin,
        move |ctx: RequestContext, input: I| {
            let checked = require_admin_user(&ctx);
            let fut = f(Arc::clone(&s), ctx, input);
            async move {
                checked?;
                fut.await
            }
        },
    )
    .summary(summary)
}

/// An admin-scope mutation that needs a recent sign-in.
fn mutation<K, I, O, F, Fut>(
    svc: &Svc<K>,
    method: HttpMethod,
    id: &'static str,
    path: &'static str,
    summary: &'static str,
    f: F,
) -> Operation
where
    K: KeyProvider + 'static,
    I: serde::de::DeserializeOwned + JsonSchema + Send + 'static,
    O: Serialize + JsonSchema + Send + 'static,
    F: Fn(Svc<K>, RequestContext, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O>> + Send + 'static,
{
    let s = Arc::clone(svc);
    Operation::mutation(
        id,
        method,
        path,
        Scope::Admin,
        move |ctx: RequestContext, input: I| {
            let checked = require_admin_user(&ctx);
            let fut = f(Arc::clone(&s), ctx, input);
            async move {
                checked?;
                fut.await
            }
        },
    )
    .summary(summary)
    .fresh_auth(FRESH_AUTH)
}

/// A long-running backup operation gets a longer budget than the default.
const BACKUP_BUDGET: Duration = Duration::from_mins(30);

impl<K: KeyProvider + 'static> Service for AdminApi<K> {
    fn name(&self) -> &'static str {
        "admin"
    }

    #[expect(clippy::too_many_lines, reason = "one declaration per operation")]
    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let svc = &self.service;
        vec![
            query(
                svc,
                "admin.list_requests",
                "/v1/admin/requests",
                "Recent requests from the trace buffer, newest first",
                |s, _ctx, input: RequestsInput| async move {
                    let query = TraceQuery {
                        route: input.route,
                        min_duration_ms: input.min_duration_ms,
                        errors_only: input.errors_only,
                        id: input.id,
                        before: input.before,
                        limit: input.limit,
                    };
                    Ok(RequestsOutput {
                        requests: s.requests(&query),
                    })
                },
            ),
            query(
                svc,
                "admin.get_request",
                "/v1/admin/requests/{trace_id}",
                "One request's trace with every span",
                |s, _ctx, input: TraceInput| async move {
                    let id: TraceId = input.trace_id.parse().map_err(|_| {
                        Error::new(ErrorKind::InvalidInput, "trace_id must be 32 hex digits")
                    })?;
                    let trace = s.request(id)?;
                    serde_json::to_value(&*trace).map_err(|e| {
                        Error::new(ErrorKind::Internal, "cannot encode the trace").with_source(e)
                    })
                },
            ),
            query(
                svc,
                "admin.routes",
                "/v1/admin/routes",
                "Rate, errors, latency and saturation per route, against its budget",
                |s, _ctx, input: WindowInput| async move { s.routes(input.minutes.unwrap_or(15)).await },
            ),
            query(
                svc,
                "admin.slos",
                "/v1/admin/slos",
                "Service level objectives, burn rates and remaining error budget",
                |s, _ctx, _input: Empty| async move { Ok(SlosOutput { slos: s.slos() }) },
            ),
            query(
                svc,
                "admin.profile",
                "/v1/admin/profile",
                "Folded span stacks of recent requests, for vellum profile",
                |s, _ctx, input: ProfileInput| async move {
                    Ok(s.profile_report(input.seconds.unwrap_or(60), input.route.as_deref()))
                },
            ),
            client_report(svc),
            query(
                svc,
                "admin.list_audit",
                "/v1/admin/audit",
                "Audit records, newest first",
                |s, _ctx, input: AuditQuery| async move {
                    Ok(AuditList {
                        records: s.audit(&input).await?,
                    })
                },
            ),
            query(
                svc,
                "admin.export_audit",
                "/v1/admin/audit/export",
                "The whole audit log as JSON Lines; the export is audited",
                |s, ctx, _input: Empty| async move {
                    Ok(AuditExport {
                        jsonl: s.export_audit(&caller(&ctx)).await?,
                    })
                },
            )
            .fresh_auth(FRESH_AUTH),
            mutation(
                svc,
                HttpMethod::Post,
                "admin.verify_audit",
                "/v1/admin/audit/verify",
                "Checks the audit log's hash chain end to end",
                |s, ctx, _input: Empty| async move { s.verify_audit(&caller(&ctx)).await },
            ),
            query(
                svc,
                "admin.get_settings",
                "/v1/admin/settings",
                "The admin settings and their version",
                |s, _ctx, _input: Empty| async move { s.settings().await },
            ),
            mutation(
                svc,
                HttpMethod::Put,
                "admin.update_settings",
                "/v1/admin/settings",
                "Replaces the settings if nobody saved since expected_version",
                |s, ctx, input: SettingsUpdate| async move {
                    s.update_settings(&caller(&ctx), input.expected_version, input.settings)
                        .await
                },
            ),
            telemetry_preview(svc, &self.facts),
            query(
                svc,
                "admin.state",
                "/v1/admin/state",
                "When backups, drills and telemetry last ran",
                |s, _ctx, _input: Empty| async move { s.state().await },
            ),
            query(
                svc,
                "admin.list_backups",
                "/v1/admin/backups",
                "Backups on disk, newest first",
                |s, _ctx, _input: Empty| async move {
                    Ok(BackupList {
                        backups: s.backups().await?,
                    })
                },
            ),
            mutation(
                svc,
                HttpMethod::Post,
                "admin.create_backup",
                "/v1/admin/backups",
                "Takes an encrypted backup now",
                |s, ctx, _input: Empty| async move { s.backup_now(&caller(&ctx)).await },
            )
            .budget(BACKUP_BUDGET),
            mutation(
                svc,
                HttpMethod::Post,
                "admin.run_drill",
                "/v1/admin/backups/drill",
                "Restores the newest backup into scratch space and checks it",
                |s, ctx, _input: Empty| async move { s.drill_now(&caller(&ctx)).await },
            )
            .budget(BACKUP_BUDGET),
        ]
    }
}

/// The browser's report: any signed-in reader may send one.
fn client_report<K: KeyProvider + 'static>(svc: &Svc<K>) -> Operation {
    let s = Arc::clone(svc);
    Operation::mutation(
        "admin.client_report",
        HttpMethod::Post,
        "/v1/telemetry/client",
        Scope::Read,
        move |_ctx: RequestContext, input: Value| {
            let s = Arc::clone(&s);
            async move {
                let body = serde_json::to_vec(&input).map_err(|e| {
                    Error::new(ErrorKind::InvalidInput, "invalid report").with_source(e)
                })?;
                let ingested = s.client_report(&body)?;
                Ok(IngestedOutput {
                    spans: ingested.spans,
                    joined: ingested.joined,
                })
            }
        },
    )
    .summary("Browser timings for a page view or slow call, joined to its trace")
}

fn telemetry_preview<K: KeyProvider + 'static>(
    svc: &Svc<K>,
    facts: &Arc<dyn UsageFactsSource>,
) -> Operation {
    let facts = Arc::clone(facts);
    query(
        svc,
        "admin.telemetry_preview",
        "/v1/admin/telemetry/preview",
        "Exactly what anonymous telemetry would send",
        move |s, _ctx, _input: Empty| {
            let facts = Arc::clone(&facts);
            async move {
                let f = facts.facts().await;
                s.telemetry_preview(&f).await
            }
        },
    )
}
