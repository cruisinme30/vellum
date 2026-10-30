// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The audit/admin application service.
//!
//! Every admin console action goes through [`AdminService`]. Reads come
//! from the in-process observability data; every change (settings,
//! backups, drills, telemetry) is written to the audit log with the actor,
//! request id and trace id, whether it succeeded or failed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use vellum_crypto::KeyProvider;
use vellum_kernel::{Error, ErrorKind, Result, Timestamp};
use vellum_observe::client::Ingested;
use vellum_observe::process::{self, ProcessSnapshot};
use vellum_observe::slo::{self, Slo, SloStatus};
use vellum_observe::store::StoreStats;
use vellum_observe::{
    ClientIngest, ClientReport, RouteMetrics, RouteStat, Trace, TraceId,
    TraceQuery, TraceStore, TraceSummary, flame,
};

use crate::audit::{
    AuditActor, AuditLog, AuditQuery, AuditRecord, ChainReport, NewAuditEvent,
    Outcome,
};
use crate::backup::{self, BackupFile, BackupInfo, DrillReport};
use crate::profile::ProfileResponse;
use crate::settings::{self, AdminSettings, SettingsStore, Versioned};
use crate::usage::{self, UsageFacts, UsageReport, UsageTransport};

/// Who is asking, for the audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// The actor.
    pub actor: AuditActor,
    /// The request id.
    pub request_id: Option<String>,
    /// The W3C trace id.
    pub trace_id: Option<String>,
}

impl Caller {
    /// Vellum itself, for scheduled work.
    #[must_use]
    pub fn system() -> Self {
        Self {
            actor: AuditActor::system(),
            request_id: None,
            trace_id: None,
        }
    }

    pub(crate) fn event(&self, action: &str) -> NewAuditEvent {
        let mut e = NewAuditEvent::new(self.actor.clone(), action);
        e.request_id.clone_from(&self.request_id);
        e.trace_id.clone_from(&self.trace_id);
        e
    }
}

/// Live RED and saturation figures for the console.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RoutesView {
    /// The window in minutes.
    pub minutes: u64,
    /// Per-route figures, busiest first.
    #[schemars(with = "Vec<serde_json::Value>")]
    pub routes: Vec<RouteStat>,
    /// Requests in progress.
    pub in_flight: i64,
    /// Process memory and CPU.
    #[schemars(with = "serde_json::Value")]
    pub process: ProcessSnapshot,
    /// The ring buffer's counters.
    #[schemars(with = "serde_json::Value")]
    pub buffer: StoreStats,
}

/// Persistent scheduler state, kept in `.vellum/admin/state.json`.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema,
)]
#[serde(default)]
pub struct AdminState {
    /// The last successful backup.
    pub last_backup: Option<BackupInfo>,
    /// The last restore drill, passed or not.
    pub last_drill: Option<DrillReport>,
    /// When anonymous telemetry was last sent, Unix milliseconds.
    pub last_telemetry_ms: Option<i64>,
}

/// What a scheduler tick did.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema,
)]
pub struct TickReport {
    /// A backup that ran.
    pub backup: Option<BackupInfo>,
    /// A drill that ran.
    pub drill: Option<DrillReport>,
    /// Whether telemetry was sent.
    pub telemetry_sent: bool,
}

/// Where the service keeps and finds things.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminPaths {
    /// The data directory (the root of every drive).
    pub data_dir: PathBuf,
    /// Scratch space for restore drills.
    pub scratch_dir: PathBuf,
}

impl AdminPaths {
    fn state_file(&self) -> PathBuf {
        self.data_dir
            .join(".vellum")
            .join("admin")
            .join("state.json")
    }

    fn backup_dir(&self, settings: &AdminSettings) -> PathBuf {
        let dest = Path::new(&settings.backups.destination);
        if dest.is_absolute() {
            dest.to_path_buf()
        } else {
            self.data_dir.join(".vellum").join(dest)
        }
    }
}

/// The audit/admin service.
pub struct AdminService<K: KeyProvider> {
    audit: Arc<dyn AuditLog>,
    settings: Arc<dyn SettingsStore>,
    backup_keys: Arc<K>,
    traces: Arc<TraceStore>,
    metrics: RouteMetrics,
    client: ClientIngest,
    slos: Vec<Slo>,
    paths: AdminPaths,
    usage: Option<Arc<dyn UsageTransport>>,
    state: Mutex<()>,
    version: String,
}

impl<K: KeyProvider> std::fmt::Debug for AdminService<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminService")
            .field("paths", &self.paths)
            .finish_non_exhaustive()
    }
}

fn internal(what: &'static str) -> impl FnOnce(serde_json::Error) -> Error {
    move |e| Error::new(ErrorKind::Internal, what).with_source(e)
}

impl<K: KeyProvider + 'static> AdminService<K> {
    /// Builds the service. `traces` and `metrics` are the same instances
    /// the recorder layer and the gateway write to.
    ///
    /// `backup_keys` must hold the backup KEK, which is not the data KEK:
    /// backups leave the server, so a copy of the server's key directory
    /// must not open them, and losing the server's keys must not lose the
    /// backups (ADR 0021, D93).
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "explicit wiring of every port"
    )]
    pub fn new(
        audit: Arc<dyn AuditLog>,
        settings: Arc<dyn SettingsStore>,
        backup_keys: Arc<K>,
        traces: Arc<TraceStore>,
        metrics: RouteMetrics,
        paths: AdminPaths,
        usage: Option<Arc<dyn UsageTransport>>,
        version: &str,
    ) -> Self {
        Self {
            audit,
            settings,
            backup_keys,
            client: ClientIngest::new(Arc::clone(&traces)),
            traces,
            metrics,
            slos: slo::default_slos(),
            paths,
            usage,
            state: Mutex::new(()),
            version: version.to_owned(),
        }
    }

    /// The audit log, for auditing the admin actions other services serve
    /// (see [`crate::audited`]) and other actions in the same chain (MCP
    /// calls, ADR 0019 D217).
    #[must_use]
    pub fn audit_log(&self) -> Arc<dyn AuditLog> {
        Arc::clone(&self.audit)
    }

    // ----- Admin → Requests -------------------------------------------------

    /// Recent requests, newest first.
    #[must_use]
    pub fn requests(&self, query: &TraceQuery) -> Vec<TraceSummary> {
        self.traces.query(query)
    }

    /// One request with every span, for the waterfall.
    ///
    /// # Errors
    ///
    /// Returns not found when the trace has left the ring buffer.
    pub fn request(&self, trace_id: TraceId) -> Result<Arc<Trace>> {
        self.traces.get(trace_id).ok_or_else(|| {
            Error::new(
                ErrorKind::NotFound,
                "that request is no longer in the buffer",
            )
        })
    }

    /// RED and saturation per route over the last `minutes`, each route
    /// marked against its p95 budget.
    ///
    /// # Errors
    ///
    /// Returns an error when the settings (which hold the budgets) cannot
    /// be read.
    pub async fn routes(&self, minutes: u64) -> Result<RoutesView> {
        let minutes = minutes.clamp(1, vellum_observe::red::WINDOW_MINUTES);
        let budgets =
            self.settings.load().await?.settings.observability.budgets;
        let mut routes = self.metrics.snapshot(minutes);
        budgets.annotate(&mut routes);
        Ok(RoutesView {
            minutes,
            routes,
            in_flight: self.metrics.in_flight(),
            process: process::snapshot(),
            buffer: self.traces.stats(),
        })
    }

    /// Every SLO with its burn rates and remaining budget.
    #[must_use]
    pub fn slos(&self) -> Vec<SloStatus> {
        slo::evaluate(&self.slos, &self.metrics)
    }

    /// Folded stacks of the traces from the last `seconds`, optionally for
    /// one route.
    #[must_use]
    pub fn profile(&self, seconds: u64, route: Option<&str>) -> flame::Folded {
        self.profile_report(seconds, route).folded_stacks()
    }

    /// [`Self::profile`] as the profile endpoint answers it, with the
    /// number of requests included.
    #[must_use]
    pub fn profile_report(
        &self,
        seconds: u64,
        route: Option<&str>,
    ) -> ProfileResponse {
        let now_ms = u64::try_from(Timestamp::now().unix_ms()).unwrap_or(0);
        let since = now_ms.saturating_sub(seconds.saturating_mul(1000));
        let traces = self.traces.since(since);
        let selected = traces.iter().map(AsRef::as_ref).filter(|t| {
            route.is_none_or(|r| {
                t.root()
                    .and_then(|s| s.field("http.route"))
                    .is_some_and(|x| x == r)
            })
        });
        let selected: Vec<&Trace> = selected.collect();
        ProfileResponse {
            folded: flame::to_text(&flame::fold(selected.iter().copied())),
            traces: selected.len(),
        }
    }

    /// Joins a browser report to its server trace.
    ///
    /// # Errors
    ///
    /// Returns invalid input when the report is malformed or out of range.
    pub fn client_report(&self, body: &[u8]) -> Result<Ingested> {
        let report = ClientReport::parse(body)
            .map_err(|e| Error::new(ErrorKind::InvalidInput, e.to_string()))?;
        Ok(self.client.ingest(&report))
    }

    // ----- Audit log --------------------------------------------------------

    /// The whole audit log as JSON Lines, oldest first, for export to a
    /// SIEM or an auditor; the export is itself audited.
    ///
    /// # Errors
    ///
    /// Returns an error when the log cannot be read.
    pub async fn export_audit(&self, caller: &Caller) -> Result<String> {
        let mut records = Vec::new();
        let mut before = None;
        loop {
            let page = self
                .audit
                .list(&AuditQuery {
                    before,
                    limit: Some(crate::audit::MAX_LIST),
                    ..AuditQuery::default()
                })
                .await?;
            let Some(last) = page.last() else { break };
            before = Some(last.seq);
            let done = page.len() < crate::audit::MAX_LIST;
            records.extend(page);
            if done {
                break;
            }
        }
        records.reverse();
        let mut out = String::new();
        for r in &records {
            out.push_str(
                &serde_json::to_string(r)
                    .map_err(internal("cannot encode audit record"))?,
            );
            out.push('\n');
        }
        let _ = self
            .audit
            .append(
                caller
                    .event("admin.audit_export")
                    .detail("records", records.len()),
            )
            .await?;
        Ok(out)
    }

    /// Audit records, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error when the log cannot be read.
    pub async fn audit(&self, query: &AuditQuery) -> Result<Vec<AuditRecord>> {
        self.audit.list(query).await
    }

    /// Checks the audit chain; the check itself is audited.
    ///
    /// # Errors
    ///
    /// Returns an error when the log cannot be read.
    pub async fn verify_audit(&self, caller: &Caller) -> Result<ChainReport> {
        let report = self.audit.verify().await?;
        let outcome = if report.broken_at.is_some() {
            Outcome::Failure
        } else {
            Outcome::Success
        };
        let _ = self
            .audit
            .append(
                caller
                    .event("admin.audit_verify")
                    .outcome(outcome)
                    .detail("records", report.records),
            )
            .await?;
        Ok(report)
    }

    // ----- Settings ---------------------------------------------------------

    /// The current settings.
    ///
    /// # Errors
    ///
    /// Returns an error when the settings cannot be read.
    pub async fn settings(&self) -> Result<Versioned> {
        self.settings.load().await
    }

    /// Replaces the settings if they are still at `expected_version`, and
    /// audits which fields changed (values included: settings hold no
    /// secrets).
    ///
    /// # Errors
    ///
    /// Returns invalid input for bad values and a conflict when someone
    /// else saved first; both are audited as failures.
    pub async fn update_settings(
        &self,
        caller: &Caller,
        expected_version: u64,
        new: AdminSettings,
    ) -> Result<Versioned> {
        let current = self.settings.load().await?;
        let changes = settings::diff(&current.settings, &new);
        let fields: Vec<String> =
            changes.iter().map(|c| c.path.clone()).collect();
        let result = self.settings.save(expected_version, &new).await;
        let event = caller
            .event("admin.settings_update")
            .target("settings")
            .detail("fields", fields)
            .detail(
                "changes",
                serde_json::to_value(&changes)
                    .map_err(internal("cannot encode changes"))?,
            );
        match result {
            Ok(version) => {
                let _ =
                    self.audit.append(event.detail("version", version)).await?;
                Ok(Versioned {
                    version,
                    settings: new,
                })
            }
            Err(e) => {
                let _ = self
                    .audit
                    .append(
                        event
                            .outcome(Outcome::Failure)
                            .detail("error", e.kind().code()),
                    )
                    .await?;
                Err(e)
            }
        }
    }

    /// The exact anonymous report that would be sent for `facts`.
    ///
    /// # Errors
    ///
    /// Returns an error when the settings cannot be read.
    pub async fn telemetry_preview(
        &self,
        facts: &UsageFacts,
    ) -> Result<UsageReport> {
        let s = self.settings.load().await?;
        Ok(usage::build_report(
            &s.settings.telemetry,
            facts,
            &self.version,
        ))
    }

    // ----- Backups ----------------------------------------------------------

    async fn load_state(&self) -> Result<AdminState> {
        match tokio::fs::read(self.paths.state_file()).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(internal("admin state is damaged")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(AdminState::default())
            }
            Err(e) => Err(Error::new(
                ErrorKind::Unavailable,
                "cannot read admin state",
            )
            .with_source(e)),
        }
    }

    async fn save_state(&self, state: &AdminState) -> Result<()> {
        let path = self.paths.state_file();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                Error::new(
                    ErrorKind::Unavailable,
                    "cannot create the admin directory",
                )
                .with_source(e)
            })?;
        }
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(internal("cannot encode admin state"))?;
        crate::fsutil::write_atomic(&path, &bytes).await
    }

    /// The scheduler state: last backup, last drill, last telemetry.
    ///
    /// # Errors
    ///
    /// Returns an error when the state file cannot be read.
    pub async fn state(&self) -> Result<AdminState> {
        self.load_state().await
    }

    /// Backups on disk, newest first.
    ///
    /// # Errors
    ///
    /// Returns an error when the backup directory cannot be read.
    pub async fn backups(&self) -> Result<Vec<BackupFile>> {
        let s = self.settings.load().await?;
        backup::list(&self.paths.backup_dir(&s.settings)).await
    }

    /// Takes a backup now, prunes old ones, and audits both.
    ///
    /// # Errors
    ///
    /// Returns the backup error, after auditing the failure.
    pub async fn backup_now(&self, caller: &Caller) -> Result<BackupInfo> {
        let _guard = self.state.lock().await;
        let s = self.settings.load().await?.settings;
        let dir = self.paths.backup_dir(&s);
        let exclude = [self.paths.scratch_dir.clone()];
        let result = backup::create(
            &*self.backup_keys,
            &self.paths.data_dir,
            &dir,
            &exclude,
        )
        .await;
        match result {
            Ok(info) => {
                let removed = backup::prune(
                    &dir,
                    usize::try_from(s.backups.keep).unwrap_or(1),
                )
                .await?;
                let _ = self
                    .audit
                    .append(
                        caller
                            .event("admin.backup_create")
                            .target(info.name.clone())
                            .detail("files", info.files)
                            .detail("bytes", info.bytes)
                            .detail("pruned", removed),
                    )
                    .await?;
                let mut state = self.load_state().await?;
                state.last_backup = Some(info.clone());
                self.save_state(&state).await?;
                Ok(info)
            }
            Err(e) => {
                let _ = self
                    .audit
                    .append(
                        caller
                            .event("admin.backup_create")
                            .outcome(Outcome::Failure)
                            .detail("error", e.message().to_owned()),
                    )
                    .await?;
                Err(e)
            }
        }
    }

    /// Restores the newest backup into scratch space, verifies it, and
    /// audits the result.
    ///
    /// # Errors
    ///
    /// Returns not found when there is no backup to drill. A failed drill
    /// is returned as a report with `passed: false`.
    pub async fn drill_now(&self, caller: &Caller) -> Result<DrillReport> {
        let _guard = self.state.lock().await;
        let s = self.settings.load().await?.settings;
        let dir = self.paths.backup_dir(&s);
        let newest =
            backup::list(&dir)
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| {
                    Error::new(
                        ErrorKind::NotFound,
                        "there is no backup to drill yet",
                    )
                })?;
        let report = backup::drill(
            &*self.backup_keys,
            &dir.join(&newest.name),
            &self.paths.scratch_dir,
        )
        .await;
        let _ = self
            .audit
            .append(
                caller
                    .event("admin.backup_drill")
                    .target(newest.name)
                    .outcome(if report.passed {
                        Outcome::Success
                    } else {
                        Outcome::Failure
                    })
                    .detail("files", report.files)
                    .detail("duration_ms", report.duration_ms)
                    .detail("within_rto", report.within_rto),
            )
            .await?;
        if !report.passed {
            tracing::error!(
                backup = %report.backup,
                error = ?report.error,
                "restore drill failed"
            );
        }
        let mut state = self.load_state().await?;
        state.last_drill = Some(report.clone());
        self.save_state(&state).await?;
        Ok(report)
    }

    /// Runs whatever is due: a backup, a restore drill, a telemetry report.
    /// Call it from the jobs runner every few minutes.
    ///
    /// # Errors
    ///
    /// Returns the first error from a due task.
    pub async fn tick(&self, facts: &UsageFacts) -> Result<TickReport> {
        let now = Timestamp::now().unix_ms();
        let s = self.settings.load().await?.settings;
        let state = self.load_state().await?;
        let mut report = TickReport::default();
        let system = Caller::system();
        let backup_due = s.backups.enabled
            && state.last_backup.as_ref().is_none_or(|b| {
                now - b.created_at.unix_ms()
                    >= i64::from(s.backups.interval_hours) * 3_600_000
            });
        if backup_due {
            report.backup = Some(self.backup_now(&system).await?);
        }
        let last_drill = state.last_drill.as_ref().map(|d| d.ran_at.unix_ms());
        if s.backups.enabled
            && backup::drill_due(last_drill, now, s.backups.drill_interval_days)
        {
            match self.drill_now(&system).await {
                Ok(d) => report.drill = Some(d),
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        if let Some(transport) = &self.usage
            && usage::is_due(&s.telemetry, state.last_telemetry_ms, now)
            && let Some(endpoint) = &s.telemetry.endpoint
        {
            let body = usage::build_report(&s.telemetry, facts, &self.version);
            transport.send(endpoint, &body).await?;
            let _ = self
                .audit
                .append(
                    system.event("admin.telemetry_send").detail(
                        "report",
                        serde_json::to_value(&body)
                            .map_err(internal("cannot encode report"))?,
                    ),
                )
                .await?;
            let mut state = self.load_state().await?;
            state.last_telemetry_ms = Some(now);
            self.save_state(&state).await?;
            report.telemetry_sent = true;
        }
        Ok(report)
    }
}
