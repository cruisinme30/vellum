// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The registry of services and their operations, and the call pipeline
//! shared by REST and `GraphQL`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use serde_json::Value;
use vellum_contracts::{Operation, RequestContext, Service};
use vellum_kernel::{Error, ErrorKind, Timestamp};

use crate::breaker::{BreakerPolicy, CircuitBreaker, is_failure};

/// An operation and the service that serves it.
#[derive(Debug)]
pub struct RegisteredOp {
    /// The service name.
    pub service: &'static str,
    /// The operation.
    pub op: Operation,
    breaker: Arc<CircuitBreaker>,
}

impl RegisteredOp {
    /// Runs the operation through the gateway's checks: scope, freshness,
    /// circuit breaker and time budget.
    ///
    /// # Errors
    ///
    /// The handler's error, or `unauthenticated`, `permission_denied`,
    /// `unavailable` (breaker open) or `timeout` from the checks.
    pub async fn invoke(&self, ctx: RequestContext, input: Value) -> Result<Value, Error> {
        let op = &self.op;
        let anonymous = ctx.principal.kind() == vellum_contracts::CredentialKind::Anonymous;
        if !(anonymous && op.allows_anonymous()) {
            ctx.principal.require_scope(op.scope())?;
        }
        if let Some(window) = op.fresh_auth_window() {
            ctx.principal.require_fresh(window, Timestamp::now())?;
        }
        if !self.breaker.try_acquire(Instant::now()) {
            return Err(Error::new(
                ErrorKind::Unavailable,
                format!("the {} service is temporarily unavailable", self.service),
            ));
        }
        let span = tracing::info_span!(
            "vellum.operation",
            operation = op.id(),
            service = self.service
        );
        let budget = op.time_budget();
        let result = tracing::Instrument::instrument(
            tokio::time::timeout(budget, op.call(ctx, input)),
            span,
        )
        .await
        .unwrap_or_else(|_| {
            Err(Error::new(
                ErrorKind::Timeout,
                format!(
                    "{} did not finish within {} ms",
                    op.id(),
                    budget.as_millis()
                ),
            ))
        });
        let failed = result.as_ref().err().is_some_and(|e| is_failure(e.kind()));
        self.breaker.record(!failed, Instant::now());
        result
    }
}

/// Every service the gateway serves.
pub struct Registry {
    services: Vec<Arc<dyn Service>>,
    ops: Vec<Arc<RegisteredOp>>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field(
                "services",
                &self.services.iter().map(|s| s.name()).collect::<Vec<_>>(),
            )
            .field("operations", &self.ops.len())
            .finish()
    }
}

/// The `GraphQL` field name for an operation ID: `pages.create_draft` →
/// `pagesCreateDraft`.
#[must_use]
pub fn graphql_name(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    let mut upper = false;
    for c in id.chars() {
        if c == '.' || c == '_' {
            upper = true;
        } else if upper {
            out.push(c.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn route_shape(method: &str, path: &str) -> String {
    let segments: Vec<&str> = path
        .split('/')
        .map(|s| if s.starts_with('{') { "{}" } else { s })
        .collect();
    format!("{method} {}", segments.join("/"))
}

impl Registry {
    /// Collects and validates every service's operations.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] for an invalid operation, a duplicate service
    /// name, operation ID, `GraphQL` name or route.
    pub fn new(services: Vec<Arc<dyn Service>>, policy: BreakerPolicy) -> Result<Self, Error> {
        let mut names = HashSet::new();
        let mut ids = HashSet::new();
        let mut graphql = HashSet::new();
        let mut routes = HashMap::new();
        let mut ops = Vec::new();
        for service in &services {
            let name = service.name();
            if !names.insert(name) {
                return Err(Error::new(
                    ErrorKind::Internal,
                    format!("service `{name}` registered twice"),
                ));
            }
            let breaker = Arc::new(CircuitBreaker::new(policy));
            for op in Arc::clone(service).operations() {
                op.validate(name)?;
                if !ids.insert(op.id()) || !graphql.insert(graphql_name(op.id())) {
                    return Err(Error::new(
                        ErrorKind::Internal,
                        format!("operation `{}` registered twice", op.id()),
                    ));
                }
                let shape = route_shape(op.method().as_str(), op.path());
                if let Some(other) = routes.insert(shape, op.id()) {
                    return Err(Error::new(
                        ErrorKind::Internal,
                        format!("operations `{other}` and `{}` share a route", op.id()),
                    ));
                }
                ops.push(Arc::new(RegisteredOp {
                    service: name,
                    op,
                    breaker: Arc::clone(&breaker),
                }));
            }
        }
        Ok(Self { services, ops })
    }

    /// The registered services.
    #[must_use]
    pub fn services(&self) -> &[Arc<dyn Service>] {
        &self.services
    }

    /// Every operation, in registration order.
    #[must_use]
    pub fn operations(&self) -> &[Arc<RegisteredOp>] {
        &self.ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(graphql_name("pages.create_draft"), "pagesCreateDraft");
        assert_eq!(graphql_name("auth.me"), "authMe");
        assert_eq!(
            route_shape("GET", "/v1/a/{x}/b"),
            route_shape("GET", "/v1/a/{y}/b")
        );
    }
}
