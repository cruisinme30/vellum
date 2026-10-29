// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The admin operations on the gateway seam.

#![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vellum_admin::api::{AdminApi, NoFacts, caller};
use vellum_admin::{AdminPaths, AdminService, MemoryAuditLog, MemorySettingsStore};
use vellum_contracts::{
    CredentialKind, Operation, OperationKind, Principal, RequestContext, Scope, ScopeSet, Service,
};
use vellum_crypto::{KekId, LocalKeyring, SecretKey};
use vellum_kernel::{ErrorKind, RequestId, Timestamp, UserId};
use vellum_observe::{RouteMetrics, TraceStore};

struct Fixture {
    operations: Vec<Operation>,
    _data: tempfile::TempDir,
    _scratch: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let data = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    std::fs::write(data.path().join("Plan.md"), "# Plan\n").unwrap();
    let service = AdminService::new(
        Arc::new(MemoryAuditLog::default()),
        Arc::new(MemorySettingsStore::default()),
        Arc::new(LocalKeyring::new(
            KekId::new("backup").unwrap(),
            SecretKey::from_bytes([7; 32]),
        )),
        Arc::new(TraceStore::default()),
        RouteMetrics::new(),
        AdminPaths {
            data_dir: data.path().to_path_buf(),
            scratch_dir: scratch.path().to_path_buf(),
        },
        None,
        "1.0.0-test",
    );
    let api = Arc::new(AdminApi::new(Arc::new(service), Arc::new(NoFacts)));
    assert_eq!(api.name(), "admin");
    Fixture {
        operations: api.operations(),
        _data: data,
        _scratch: scratch,
    }
}

fn ctx(kind: CredentialKind, scopes: &[Scope]) -> RequestContext {
    RequestContext {
        request_id: RequestId::generate(),
        principal: Principal::user(
            UserId::generate(),
            kind,
            ScopeSet::from_scopes(scopes.iter().copied()),
            Timestamp::now(),
        ),
        deadline: Instant::now() + Duration::from_secs(30),
        traceparent: Some("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01".into()),
    }
}

fn admin() -> RequestContext {
    ctx(CredentialKind::Session, &[Scope::Admin])
}

impl Fixture {
    fn op(&self, id: &str) -> &Operation {
        self.operations.iter().find(|o| o.id() == id).unwrap()
    }

    async fn call(&self, id: &str, input: Value) -> vellum_kernel::Result<Value> {
        self.op(id).call(admin(), input).await
    }
}

#[test]
fn declares_valid_operations() {
    let f = fixture();
    assert_eq!(f.operations.len(), 16);
    for op in &f.operations {
        op.validate("admin").unwrap();
        if op.id() == "admin.client_report" {
            assert_eq!(op.scope(), Scope::Read);
            assert_eq!(op.path(), "/v1/telemetry/client");
        } else {
            assert_eq!(op.scope(), Scope::Admin, "{}", op.id());
        }
        if op.kind() == OperationKind::Mutation && op.scope() == Scope::Admin {
            assert!(op.fresh_auth_window().is_some(), "{}", op.id());
        }
    }
    assert!(f.op("admin.export_audit").fresh_auth_window().is_some());
}

#[tokio::test]
async fn serves_the_console() {
    let f = fixture();
    let requests = f
        .call("admin.list_requests", json!({"limit": 5}))
        .await
        .unwrap();
    assert_eq!(requests, json!({"requests": []}));
    let missing = f
        .call(
            "admin.get_request",
            json!({"trace_id": "4bf92f3577b34da6a3ce929d0e0e4736"}),
        )
        .await
        .unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NotFound);
    let bad = f
        .call("admin.get_request", json!({"trace_id": "nope"}))
        .await
        .unwrap_err();
    assert_eq!(bad.kind(), ErrorKind::InvalidInput);
    let routes = f.call("admin.routes", json!({})).await.unwrap();
    assert_eq!(routes["minutes"], 15);
    let slos = f.call("admin.slos", json!({})).await.unwrap();
    assert_eq!(slos["slos"].as_array().unwrap().len(), 3);
    let profile = f
        .call("admin.profile", json!({"seconds": 5}))
        .await
        .unwrap();
    assert_eq!(profile["traces"], 0);
    let report = json!({
        "traceparent": "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        "page": "/p/x", "request_start_ms": 1.0, "time_origin_ms": 1.0,
        "spans": [{"name": "dom", "start_ms": 1.0, "duration_ms": 2.0}]
    });
    let ingested = f
        .op("admin.client_report")
        .call(ctx(CredentialKind::Session, &[Scope::Read]), report)
        .await
        .unwrap();
    assert_eq!(ingested["joined"], false);
    let preview = f.call("admin.telemetry_preview", json!({})).await.unwrap();
    assert!(preview.is_object());
    let state = f.call("admin.state", json!({})).await.unwrap();
    assert!(state.is_object());
}

#[tokio::test]
async fn audits_settings_and_backups() {
    let f = fixture();
    let current = f.call("admin.get_settings", json!({})).await.unwrap();
    let mut settings = current["settings"].clone();
    settings["organisation"] = json!("Northstar");
    let saved = f
        .call(
            "admin.update_settings",
            json!({"expected_version": current["version"], "settings": settings}),
        )
        .await
        .unwrap();
    assert_eq!(saved["settings"]["organisation"], "Northstar");
    let stale = f
        .call(
            "admin.update_settings",
            json!({"expected_version": current["version"], "settings": settings}),
        )
        .await
        .unwrap_err();
    assert_eq!(stale.kind(), ErrorKind::Conflict);

    let backup = f.call("admin.create_backup", json!({})).await.unwrap();
    assert_eq!(backup["files"], 1);
    let backups = f.call("admin.list_backups", json!({})).await.unwrap();
    assert_eq!(backups["backups"].as_array().unwrap().len(), 1);
    let drill = f.call("admin.run_drill", json!({})).await.unwrap();
    assert_eq!(drill["passed"], true);

    let records = f.call("admin.list_audit", json!({})).await.unwrap();
    let records = records["records"].as_array().unwrap();
    assert!(records.len() >= 4);
    let actor = &records.first().unwrap()["event"]["actor"];
    assert_eq!(actor["kind"], "user");
    assert_eq!(actor["via"], "session");
    let chain = f.call("admin.verify_audit", json!({})).await.unwrap();
    assert!(chain["broken_at"].is_null());
    let export = f.call("admin.export_audit", json!({})).await.unwrap();
    assert!(export["jsonl"].as_str().unwrap().lines().count() >= 5);
}

#[tokio::test]
async fn refuses_callers_without_the_admin_scope() {
    let f = fixture();
    let reader = ctx(CredentialKind::ApiToken, &[Scope::Read]);
    let denied = f
        .op("admin.get_settings")
        .call(reader.clone(), json!({}))
        .await
        .unwrap_err();
    assert_eq!(denied.kind(), ErrorKind::PermissionDenied);
    let c = caller(&reader);
    assert_eq!(c.actor.kind, "api_token");
    assert_eq!(
        c.trace_id.as_deref(),
        Some("4bf92f3577b34da6a3ce929d0e0e4736")
    );
    let share = RequestContext {
        principal: Principal::anonymous(),
        traceparent: None,
        ..reader
    };
    let c = caller(&share);
    assert_eq!(c.actor.kind, "external");
    assert!(c.trace_id.is_none());
}
