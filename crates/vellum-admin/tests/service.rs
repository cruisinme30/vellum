// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The admin service wired to in-memory ports.

#![allow(
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    reason = "test fixtures fail loudly"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use tracing_subscriber::layer::SubscriberExt as _;
use vellum_admin::AuditLog as _;
use vellum_admin::audit::AuditQuery;
use vellum_admin::usage::{UsageFacts, UsageReport, UsageTransport};
use vellum_admin::{
    AdminPaths, AdminService, AuditActor, Caller, MemoryAuditLog, MemorySettingsStore,
};
use vellum_crypto::{KekId, LocalKeyring, SecretKey};
use vellum_kernel::{ErrorKind, Result};
use vellum_observe::{RequestObservation, RouteMetrics, TraceParent, TraceQuery, TraceStore};

#[derive(Default)]
struct Recorded(Mutex<Vec<(String, UsageReport)>>);

#[async_trait]
impl UsageTransport for Recorded {
    async fn send(&self, endpoint: &str, report: &UsageReport) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .push((endpoint.to_owned(), report.clone()));
        Ok(())
    }
}

struct Fixture {
    service: AdminService<LocalKeyring>,
    audit: Arc<MemoryAuditLog>,
    traces: Arc<TraceStore>,
    metrics: RouteMetrics,
    transport: Arc<Recorded>,
    _data: tempfile::TempDir,
    _scratch: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let data = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(data.path().join("drives/shared/eng")).unwrap();
    std::fs::write(data.path().join("drives/shared/eng/Plan.md"), "# Plan\n").unwrap();
    let audit = Arc::new(MemoryAuditLog::default());
    let traces = Arc::new(TraceStore::default());
    let metrics = RouteMetrics::new();
    let transport = Arc::new(Recorded::default());
    let keys = Arc::new(LocalKeyring::new(
        KekId::new("k").unwrap(),
        SecretKey::from_bytes([9; 32]),
    ));
    let service = AdminService::new(
        audit.clone(),
        Arc::new(MemorySettingsStore::default()),
        keys,
        traces.clone(),
        metrics.clone(),
        AdminPaths {
            data_dir: data.path().to_path_buf(),
            scratch_dir: scratch.path().to_path_buf(),
        },
        Some(transport.clone() as Arc<dyn UsageTransport>),
        "1.0.0-test",
    );
    Fixture {
        service,
        audit,
        traces,
        metrics,
        transport,
        _data: data,
        _scratch: scratch,
    }
}

fn admin() -> Caller {
    Caller {
        actor: AuditActor::user("01J9ADMIN"),
        request_id: Some("req-7".into()),
        trace_id: None,
    }
}

#[tokio::test]
async fn settings_changes_are_versioned_and_audited() {
    let f = fixture();
    let current = f.service.settings().await.unwrap();
    let mut next = current.settings.clone();
    next.organisation = "Northstar".into();
    next.backups.keep = 7;
    let saved = f
        .service
        .update_settings(&admin(), 0, next.clone())
        .await
        .unwrap();
    assert_eq!(saved.version, 1);
    let err = f
        .service
        .update_settings(&admin(), 0, next)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Conflict);
    let records = f.service.audit(&AuditQuery::default()).await.unwrap();
    assert_eq!(records.len(), 2);
    let ok = records.last().unwrap();
    assert_eq!(ok.event.action, "admin.settings_update");
    assert_eq!(ok.event.request_id.as_deref(), Some("req-7"));
    let fields = ok.event.details.get("fields").unwrap().to_string();
    assert!(fields.contains("organisation") && fields.contains("backups.keep"));
    assert_eq!(
        records.first().unwrap().event.outcome,
        vellum_admin::Outcome::Failure
    );
    let export = f.service.export_audit(&admin()).await.unwrap();
    let lines: Vec<&str> = export.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("\"seq\":1"));
    let chain = f.service.verify_audit(&admin()).await.unwrap();
    assert_eq!(chain.records, 3);
    assert!(chain.broken_at.is_none());
}

#[tokio::test]
async fn tick_backs_up_drills_and_reports_once() {
    let f = fixture();
    let err = f.service.drill_now(&admin()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);

    let mut s = f.service.settings().await.unwrap().settings;
    s.telemetry.enabled = true;
    s.telemetry.endpoint = Some("https://telemetry.example/v1".into());
    let _ = f.service.update_settings(&admin(), 0, s).await.unwrap();

    let facts = UsageFacts {
        users: 3,
        profile: "single".into(),
        database: "sqlite".into(),
        ..UsageFacts::default()
    };
    let preview = f.service.telemetry_preview(&facts).await.unwrap();
    assert_eq!(preview.users, "2-10");

    let first = f.service.tick(&facts).await.unwrap();
    assert!(first.backup.is_some());
    assert!(first.drill.as_ref().unwrap().passed);
    assert!(first.telemetry_sent);
    let sent = f.transport.0.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].1, preview);

    let second = f.service.tick(&facts).await.unwrap();
    assert_eq!(
        second,
        vellum_admin::service::TickReport::default(),
        "nothing is due again yet"
    );

    let state = f.service.state().await.unwrap();
    assert!(state.last_backup.is_some() && state.last_drill.is_some());
    assert_eq!(f.service.backups().await.unwrap().len(), 1);
    let actions: Vec<String> = f
        .audit
        .list(&AuditQuery::default())
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.event.action)
        .collect();
    for action in [
        "admin.backup_create",
        "admin.backup_drill",
        "admin.telemetry_send",
    ] {
        assert!(actions.iter().any(|a| a == action), "{action}");
    }
}

#[tokio::test]
async fn requests_routes_slos_profile_and_client_reports() {
    let f = fixture();
    let subscriber =
        tracing_subscriber::registry().with(vellum_observe::recorder_layer(f.traces.clone()));
    let trace_id = tracing::subscriber::with_default(subscriber, || {
        tracing::callsite::rebuild_interest_cache();
        let obs =
            RequestObservation::begin(&f.metrics, "GET", "/api/v1/pages/{page_id}", None, "req-1");
        let id = obs.trace_id();
        {
            let _enter = obs.span().enter();
            let db = tracing::info_span!("db.query");
            let _db = db.enter();
            std::thread::sleep(Duration::from_millis(2));
        }
        let _ = obs.finish(200, Vec::new());
        id
    });
    let rows = f.service.requests(&TraceQuery::default());
    assert_eq!(rows.len(), 1);
    let trace = f.service.request(trace_id).unwrap();
    assert_eq!(trace.spans.len(), 2);
    assert_eq!(
        f.service
            .request(vellum_observe::TraceId::random())
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );

    let routes = f.service.routes(5).await.unwrap();
    assert_eq!(routes.routes.len(), 1);
    let route = routes.routes.first().unwrap();
    assert_eq!(route.budget_ms, Some(150));
    assert!(!route.over_budget, "one request is too few to judge");
    assert_eq!(routes.buffer.traces, 1);
    assert_eq!(f.service.slos().len(), 3);

    let folded = f.service.profile(60, Some("/api/v1/pages/{page_id}"));
    assert!(folded.keys().any(|k| k.ends_with("db.query")));
    assert!(f.service.profile(60, Some("/other")).is_empty());

    let parent = TraceParent {
        trace_id,
        parent_id: vellum_observe::SpanId::random(),
        sampled: true,
    };
    let body = format!(
        r#"{{"traceparent":"{parent}","page":"/p/x","request_start_ms":10.0,"time_origin_ms":1.0,
            "spans":[{{"name":"fetch","start_ms":10.0,"duration_ms":30.0}}]}}"#
    );
    let ingested = f.service.client_report(body.as_bytes()).unwrap();
    assert!(ingested.joined);
    assert_eq!(f.service.request(trace_id).unwrap().client.len(), 1);
    assert_eq!(
        f.service.client_report(b"{}").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}
