// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Audits the admin actions other services serve (ADR 0021, D243).
//!
//! The admin console's own operations write their audit records
//! themselves. Users, spaces and the other admin operations live in the
//! services that own the data (identity, drive, work), which know nothing
//! of the audit log. [`audited`] wraps such a service so every operation
//! that needs the `admin` scope and changes something appends one record:
//! who, which operation, on what, and how it ended. Inputs are never
//! copied into the record, because they can hold passwords.

use std::sync::Arc;

use serde_json::Value;
use vellum_contracts::{
    BoxFuture, Handler, Operation, OperationKind, Readiness, RequestContext, Scope, Service,
};
use vellum_kernel::{Error, ErrorKind};

use crate::api::caller;
use crate::audit::{AuditLog, Outcome};

/// The input fields that name what an admin action changes, in the order
/// they are tried.
const TARGET_FIELDS: [&str; 4] = ["id", "user_id", "space_id", "member_id"];

/// Wraps `service` so its admin-scope mutations are audited in `log`.
/// Other operations pass through untouched.
#[must_use]
pub fn audited(service: Arc<dyn Service>, log: Arc<dyn AuditLog>) -> Arc<dyn Service> {
    Arc::new(Audited {
        inner: service,
        log,
    })
}

/// Whether [`audited`] records `op`.
#[must_use]
pub fn is_admin_action(op: &Operation) -> bool {
    op.scope() == Scope::Admin && op.kind() == OperationKind::Mutation
}

struct Audited {
    inner: Arc<dyn Service>,
    log: Arc<dyn AuditLog>,
}

impl Service for Audited {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        Arc::clone(&self.inner)
            .operations()
            .into_iter()
            .map(|op| {
                if !is_admin_action(&op) {
                    return op;
                }
                let action = op.id();
                let log = Arc::clone(&self.log);
                op.wrap(move |inner| Arc::new(AuditedHandler { inner, log, action }))
            })
            .collect()
    }

    fn readiness(&self) -> BoxFuture<'_, Readiness> {
        self.inner.readiness()
    }
}

struct AuditedHandler {
    inner: Arc<dyn Handler>,
    log: Arc<dyn AuditLog>,
    action: &'static str,
}

/// The first of [`TARGET_FIELDS`] the input holds as a string.
fn target_of(input: &Value) -> Option<String> {
    TARGET_FIELDS
        .iter()
        .find_map(|field| input.get(field).and_then(Value::as_str))
        .map(str::to_owned)
}

const fn outcome_of(result: &Result<Value, Error>) -> Outcome {
    match result {
        Ok(_) => Outcome::Success,
        Err(e)
            if matches!(
                e.kind(),
                ErrorKind::PermissionDenied | ErrorKind::Unauthenticated
            ) =>
        {
            Outcome::Denied
        }
        Err(_) => Outcome::Failure,
    }
}

impl Handler for AuditedHandler {
    fn call(&self, ctx: RequestContext, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
        let caller = caller(&ctx);
        let target = target_of(&input);
        let space = input
            .get("space_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let log = Arc::clone(&self.log);
        let action = self.action;
        let run = self.inner.call(ctx, input);
        Box::pin(async move {
            let result = run.await;
            let mut event = caller.event(action).outcome(outcome_of(&result));
            if let Some(target) = target {
                event = event.target(target);
            }
            if let Some(space) = space {
                event = event.detail("space_id", space);
            }
            if let Err(e) = &result {
                event = event.detail("error", e.kind().code());
            }
            // A change that happened must not be reported as failed because
            // the audit write failed; the failure is logged loudly instead.
            if let Err(e) = log.append(event).await {
                tracing::error!(action, error = %e, "could not audit an admin action");
            }
            result
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use serde::Deserialize;
    use vellum_contracts::{CredentialKind, HttpMethod, Principal, ScopeSet};
    use vellum_kernel::{RequestId, Timestamp, UserId};

    use super::*;
    use crate::audit::{AuditQuery, MemoryAuditLog};

    #[derive(Deserialize, schemars::JsonSchema)]
    struct Input {
        id: String,
        #[expect(dead_code, reason = "only here to prove it is not audited")]
        password: Option<String>,
    }

    struct Users;

    impl Service for Users {
        fn name(&self) -> &'static str {
            "users"
        }

        fn operations(self: Arc<Self>) -> Vec<Operation> {
            vec![
                Operation::mutation(
                    "users.block",
                    HttpMethod::Patch,
                    "/v1/admin/users/{id}",
                    Scope::Admin,
                    |_ctx, input: Input| async move {
                        if input.id == "missing" {
                            Err(Error::new(ErrorKind::NotFound, "user not found"))
                        } else {
                            Ok(true)
                        }
                    },
                )
                .fresh_auth(Duration::from_mins(5)),
                Operation::mutation(
                    "users.rename_me",
                    HttpMethod::Patch,
                    "/v1/me",
                    Scope::Read,
                    |_ctx, _input: Input| async move { Ok(true) },
                ),
                Operation::query(
                    "users.list",
                    "/v1/admin/users",
                    Scope::Admin,
                    |_ctx, _input: Input| async move { Ok(true) },
                ),
            ]
        }
    }

    fn ctx() -> RequestContext {
        RequestContext {
            request_id: RequestId::generate(),
            principal: Principal::user(
                UserId::generate(),
                CredentialKind::Session,
                ScopeSet::from_scopes([Scope::Admin]),
                Timestamp::now(),
            ),
            deadline: Instant::now() + Duration::from_secs(5),
            traceparent: None,
        }
    }

    #[tokio::test]
    async fn admin_mutations_are_audited_without_their_input() {
        let log = Arc::new(MemoryAuditLog::default());
        let service = audited(Arc::new(Users), Arc::clone(&log) as Arc<dyn AuditLog>);
        let ops = service.operations();
        let by_id = |id: &str| ops.iter().find(|o| o.id() == id).expect("declared");
        assert_eq!(
            by_id("users.block").fresh_auth_window(),
            Some(Duration::from_mins(5)),
            "the declaration is kept"
        );
        let input = serde_json::json!({"id": "u1", "password": "hunter2-hunter2"});
        let _ = by_id("users.block").call(ctx(), input).await.expect("ok");
        let _ = by_id("users.block")
            .call(ctx(), serde_json::json!({"id": "missing"}))
            .await
            .expect_err("not found");
        let _ = by_id("users.rename_me")
            .call(ctx(), serde_json::json!({"id": "u1"}))
            .await
            .expect("ok");
        let _ = by_id("users.list")
            .call(ctx(), serde_json::json!({"id": "u1"}))
            .await
            .expect("ok");

        let records = log.list(&AuditQuery::default()).await.expect("list");
        assert_eq!(records.len(), 2, "only the admin mutation is audited");
        let failed = &records[0].event;
        assert_eq!(failed.action, "users.block");
        assert_eq!(failed.outcome, Outcome::Failure);
        assert_eq!(failed.target.as_deref(), Some("missing"));
        let ok = &records[1].event;
        assert_eq!(ok.outcome, Outcome::Success);
        assert_eq!(ok.target.as_deref(), Some("u1"));
        assert!(ok.request_id.is_some());
        let stored = serde_json::to_string(&records).expect("json");
        assert!(!stored.contains("hunter2"), "inputs are never stored");
    }
}
