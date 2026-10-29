// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The service-registration seam between Vellum's services and the gateway.
//!
//! Each service (spaces and pages, collections, search, …) implements
//! [`Service`] and describes its API as a list of [`Operation`]s. The
//! gateway turns every operation into a REST route, an OpenAPI 3.1
//! operation and a GraphQL query or mutation, so REST and `GraphQL` always
//! cover the same mutations. The gateway, not the service, handles
//! authentication, scope checks, re-authentication, idempotency, request
//! IDs, timeouts, circuit breaking and `Server-Timing`.
//!
//! A service that needs a raw HTTP surface (`WebSocket` rooms, streamed
//! downloads, MCP) exposes its own router from its crate; the binary mounts
//! it on the gateway, which wraps it with the same middleware and puts the
//! [`Principal`] in the request extensions.
//!
//! ```
//! use std::sync::Arc;
//! use schemars::JsonSchema;
//! use serde::{Deserialize, Serialize};
//! use vellum_contracts::{Operation, Scope, Service};
//!
//! #[derive(Deserialize, JsonSchema)]
//! struct Greet { name: String }
//!
//! #[derive(Serialize, JsonSchema)]
//! struct Greeting { text: String }
//!
//! struct Hello;
//!
//! impl Service for Hello {
//!     fn name(&self) -> &'static str { "hello" }
//!     fn operations(self: Arc<Self>) -> Vec<Operation> {
//!         vec![Operation::query("hello.greet", "/v1/hello/{name}", Scope::Read,
//!             |_ctx, input: Greet| async move {
//!                 Ok(Greeting { text: format!("Hello, {}!", input.name) })
//!             })
//!             .summary("Greets someone by name")]
//!     }
//! }
//! ```

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use schemars::{JsonSchema, Schema, SchemaGenerator};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use vellum_kernel::{Error, ErrorKind, RequestId};

use crate::principal::{Principal, Scope};

/// A boxed, sendable future, as returned by the seam's object-safe traits.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The default time budget for a query.
pub const DEFAULT_QUERY_BUDGET: Duration = Duration::from_secs(5);
/// The default time budget for a mutation.
pub const DEFAULT_MUTATION_BUDGET: Duration = Duration::from_secs(10);

/// A service the gateway can expose. One binary may run any subset.
pub trait Service: Send + Sync + 'static {
    /// A stable, lowercase name (`pages`, `collections`), used as the
    /// operation-ID prefix, the OpenAPI tag, the metrics label and the
    /// circuit-breaker key.
    fn name(&self) -> &'static str;

    /// The operations this service serves. Called once at start-up;
    /// handlers capture their own clone of the service state.
    fn operations(self: Arc<Self>) -> Vec<Operation>;

    /// Whether the service can take traffic now. `/readyz` reports every
    /// service; one `NotReady` makes the process not ready.
    fn readiness(&self) -> BoxFuture<'_, Readiness> {
        Box::pin(async { Readiness::Ready })
    }
}

/// A service's answer to the readiness probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum Readiness {
    /// Serving normally.
    Ready,
    /// Serving with reduced function (blob store down: cached reads only).
    Degraded(String),
    /// Unable to serve; traffic should go elsewhere.
    NotReady(String),
}

/// Whether an operation reads or writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    /// No side effects; a GraphQL query, served by `GET`.
    Query,
    /// Changes state; a GraphQL mutation. Retries are made safe by the
    /// `Idempotency-Key` header.
    Mutation,
}

/// The HTTP method of an operation's REST route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    /// Read.
    Get,
    /// Create, or run an action.
    Post,
    /// Replace.
    Put,
    /// Partially update.
    Patch,
    /// Delete.
    Delete,
}

impl HttpMethod {
    /// The method name in upper case.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }

    /// Whether the input travels in a JSON body (otherwise in the query
    /// string).
    #[must_use]
    pub const fn has_body(self) -> bool {
        matches!(self, Self::Post | Self::Put | Self::Patch)
    }
}

/// Everything a handler knows about the request beyond its input.
#[derive(Debug, Clone)]
pub struct RequestContext {
    /// The request ID, also returned in the `X-Request-Id` header.
    pub request_id: RequestId,
    /// The authenticated caller.
    pub principal: Principal,
    /// When the gateway will abandon the request; pass it to downstream
    /// calls so they stop in time.
    pub deadline: Instant,
    /// The W3C `traceparent` of the request's span, for outbound calls.
    pub traceparent: Option<String>,
}

impl RequestContext {
    /// The time left before the deadline, zero once it has passed.
    #[must_use]
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
}

/// The type-erased handler behind an [`Operation`].
pub trait Handler: Send + Sync + 'static {
    /// Runs the operation on JSON input and returns JSON output.
    fn call(&self, ctx: RequestContext, input: Value) -> BoxFuture<'static, Result<Value, Error>>;
}

struct TypedHandler<F, I, O> {
    f: F,
    _types: std::marker::PhantomData<fn(I) -> O>,
}

impl<F, Fut, I, O> Handler for TypedHandler<F, I, O>
where
    F: Fn(RequestContext, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, Error>> + Send + 'static,
    I: DeserializeOwned + Send + 'static,
    O: Serialize + Send + 'static,
{
    fn call(&self, ctx: RequestContext, input: Value) -> BoxFuture<'static, Result<Value, Error>> {
        let parsed = serde_json::from_value::<I>(input);
        match parsed {
            Err(err) => Box::pin(std::future::ready(Err(Error::new(
                ErrorKind::InvalidInput,
                format!("invalid input: {err}"),
            )))),
            Ok(input) => {
                let fut = (self.f)(ctx, input);
                Box::pin(async move {
                    let output = fut.await?;
                    serde_json::to_value(output).map_err(|err| {
                        Error::new(ErrorKind::Internal, "could not encode the response")
                            .with_source(err)
                    })
                })
            }
        }
    }
}

/// One API operation: a REST route, an OpenAPI operation and a GraphQL
/// field, all from a single declaration.
#[derive(Clone)]
pub struct Operation {
    id: &'static str,
    kind: OperationKind,
    method: HttpMethod,
    path: &'static str,
    scope: Scope,
    anonymous: bool,
    fresh_auth: Option<Duration>,
    budget: Duration,
    summary: &'static str,
    description: &'static str,
    deprecated: bool,
    input: Schema,
    output: Schema,
    handler: Arc<dyn Handler>,
}

impl fmt::Debug for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Operation")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("method", &self.method)
            .field("path", &self.path)
            .field("scope", &self.scope)
            .finish_non_exhaustive()
    }
}

fn schema_of<T: JsonSchema>() -> Schema {
    SchemaGenerator::new(schemars::generate::SchemaSettings::draft2020_12())
        .into_root_schema_for::<T>()
}

impl Operation {
    fn build<F, Fut, I, O>(
        id: &'static str,
        kind: OperationKind,
        method: HttpMethod,
        path: &'static str,
        scope: Scope,
        handler: F,
    ) -> Self
    where
        F: Fn(RequestContext, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O, Error>> + Send + 'static,
        I: DeserializeOwned + JsonSchema + Send + 'static,
        O: Serialize + JsonSchema + Send + 'static,
    {
        let budget = match kind {
            OperationKind::Query => DEFAULT_QUERY_BUDGET,
            OperationKind::Mutation => DEFAULT_MUTATION_BUDGET,
        };
        Self {
            id,
            kind,
            method,
            path,
            scope,
            anonymous: false,
            fresh_auth: None,
            budget,
            summary: "",
            description: "",
            deprecated: false,
            input: schema_of::<I>(),
            output: schema_of::<O>(),
            handler: Arc::new(TypedHandler {
                f: handler,
                _types: std::marker::PhantomData,
            }),
        }
    }

    /// A read served by `GET path`. Path parameters (`{space_id}`) and
    /// query-string parameters are fields of the input type `I`.
    pub fn query<F, Fut, I, O>(
        id: &'static str,
        path: &'static str,
        scope: Scope,
        handler: F,
    ) -> Self
    where
        F: Fn(RequestContext, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O, Error>> + Send + 'static,
        I: DeserializeOwned + JsonSchema + Send + 'static,
        O: Serialize + JsonSchema + Send + 'static,
    {
        Self::build(
            id,
            OperationKind::Query,
            HttpMethod::Get,
            path,
            scope,
            handler,
        )
    }

    /// A write served by `method path`. Path parameters and the JSON body
    /// (or the query string for `DELETE`) are merged into the input `I`.
    pub fn mutation<F, Fut, I, O>(
        id: &'static str,
        method: HttpMethod,
        path: &'static str,
        scope: Scope,
        handler: F,
    ) -> Self
    where
        F: Fn(RequestContext, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<O, Error>> + Send + 'static,
        I: DeserializeOwned + JsonSchema + Send + 'static,
        O: Serialize + JsonSchema + Send + 'static,
    {
        Self::build(id, OperationKind::Mutation, method, path, scope, handler)
    }

    /// Sets the one-line summary shown in OpenAPI and GraphQL docs.
    #[must_use]
    pub const fn summary(mut self, summary: &'static str) -> Self {
        self.summary = summary;
        self
    }

    /// Sets the longer description.
    #[must_use]
    pub const fn description(mut self, description: &'static str) -> Self {
        self.description = description;
        self
    }

    /// Lets callers without a credential run the operation (sign-in, the
    /// public instance description). Callers that do present a credential
    /// still need `scope`.
    #[must_use]
    pub const fn allow_anonymous(mut self) -> Self {
        self.anonymous = true;
        self
    }

    /// Requires the caller to have signed in within `max_age` (admin
    /// actions). The gateway answers older sessions with `401
    /// reauthentication required`.
    #[must_use]
    pub const fn fresh_auth(mut self, max_age: Duration) -> Self {
        self.fresh_auth = Some(max_age);
        self
    }

    /// Overrides the time budget (the route's timeout).
    #[must_use]
    pub const fn budget(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }

    /// Marks the operation deprecated in OpenAPI and GraphQL.
    #[must_use]
    pub const fn deprecated(mut self) -> Self {
        self.deprecated = true;
        self
    }

    /// The stable ID, `<service>.<name>` in `snake_case` segments.
    #[must_use]
    pub const fn id(&self) -> &'static str {
        self.id
    }

    /// Query or mutation.
    #[must_use]
    pub const fn kind(&self) -> OperationKind {
        self.kind
    }

    /// The REST method.
    #[must_use]
    pub const fn method(&self) -> HttpMethod {
        self.method
    }

    /// The REST path template below `/api`, starting `/v1/`.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        self.path
    }

    /// The scope a credential needs.
    #[must_use]
    pub const fn scope(&self) -> Scope {
        self.scope
    }

    /// Whether anonymous callers may run it.
    #[must_use]
    pub const fn allows_anonymous(&self) -> bool {
        self.anonymous
    }

    /// The re-authentication window, for admin actions.
    #[must_use]
    pub const fn fresh_auth_window(&self) -> Option<Duration> {
        self.fresh_auth
    }

    /// The time budget.
    #[must_use]
    pub const fn time_budget(&self) -> Duration {
        self.budget
    }

    /// The summary.
    #[must_use]
    pub const fn summary_text(&self) -> &'static str {
        self.summary
    }

    /// The description.
    #[must_use]
    pub const fn description_text(&self) -> &'static str {
        self.description
    }

    /// Whether the operation is deprecated.
    #[must_use]
    pub const fn is_deprecated(&self) -> bool {
        self.deprecated
    }

    /// The JSON Schema (2020-12) of the input.
    #[must_use]
    pub const fn input_schema(&self) -> &Schema {
        &self.input
    }

    /// The JSON Schema (2020-12) of the output.
    #[must_use]
    pub const fn output_schema(&self) -> &Schema {
        &self.output
    }

    /// The names of the path parameters, in order.
    pub fn path_params(&self) -> impl Iterator<Item = &'static str> {
        path_params(self.path)
    }

    /// Runs the handler.
    #[must_use = "the future does nothing until awaited"]
    pub fn call(
        &self,
        ctx: RequestContext,
        input: Value,
    ) -> BoxFuture<'static, Result<Value, Error>> {
        self.handler.call(ctx, input)
    }

    /// Checks the declaration: the ID's shape, the path's shape, and that
    /// every path parameter is a field of the input.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] describing the first problem; the gateway
    /// refuses to start with an invalid operation.
    pub fn validate(&self, service: &str) -> Result<(), Error> {
        let bad = |what: String| Err(Error::new(ErrorKind::Internal, what));
        let mut segments = self.id.split('.');
        if segments.next() != Some(service) || !valid_id(self.id) {
            return bad(format!(
                "operation `{}` must be `{service}.<name>` in snake_case segments",
                self.id
            ));
        }
        if !self.path.starts_with("/v1/") || self.path.ends_with('/') {
            return bad(format!(
                "operation `{}` path must start with /v1/ and not end with /",
                self.id
            ));
        }
        if self.kind == OperationKind::Query && self.method != HttpMethod::Get {
            return bad(format!("query `{}` must use GET", self.id));
        }
        if self.kind == OperationKind::Mutation && self.method == HttpMethod::Get {
            return bad(format!("mutation `{}` must not use GET", self.id));
        }
        let properties = self.input.get("properties").and_then(Value::as_object);
        for param in self.path_params() {
            if !properties.is_some_and(|p| p.contains_key(param)) {
                return bad(format!(
                    "operation `{}` path parameter `{param}` is not a field of its input",
                    self.id
                ));
            }
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    let segments: Vec<&str> = id.split('.').collect();
    segments.len() >= 2
        && segments.iter().all(|seg| {
            let mut chars = seg.chars();
            chars.next().is_some_and(|c| c.is_ascii_lowercase())
                && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

/// The `{name}` parameters of a path template, in order.
#[must_use = "the iterator is lazy"]
pub fn path_params(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter_map(|seg| {
        seg.strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
    })
}

#[cfg(test)]
mod tests {
    use schemars::JsonSchema;
    use serde::Deserialize;
    use vellum_kernel::{Timestamp, UserId};

    use super::*;
    use crate::principal::{CredentialKind, ScopeSet};

    #[derive(Deserialize, JsonSchema)]
    struct Input {
        #[expect(dead_code, reason = "only its schema is used")]
        space_id: String,
        count: u32,
    }

    #[derive(Serialize, JsonSchema)]
    struct Output {
        total: u32,
    }

    fn op(id: &'static str, path: &'static str) -> Operation {
        Operation::query(id, path, Scope::Read, |_ctx, input: Input| async move {
            Ok(Output {
                total: input.count * 2,
            })
        })
    }

    fn ctx() -> RequestContext {
        RequestContext {
            request_id: RequestId::generate(),
            principal: Principal::user(
                UserId::generate(),
                CredentialKind::Session,
                ScopeSet::FULL,
                Timestamp::now(),
            ),
            deadline: Instant::now() + Duration::from_secs(1),
            traceparent: None,
        }
    }

    #[test]
    fn validation() {
        assert!(
            op("pages.count", "/v1/spaces/{space_id}/count")
                .validate("pages")
                .is_ok()
        );
        assert!(
            op("pages.count", "/v1/spaces/{space_id}")
                .validate("search")
                .is_err()
        );
        assert!(
            op("pages.Count", "/v1/spaces/{space_id}")
                .validate("pages")
                .is_err()
        );
        assert!(
            op("pages", "/v1/spaces/{space_id}")
                .validate("pages")
                .is_err()
        );
        assert!(
            op("pages.count", "/spaces/{space_id}")
                .validate("pages")
                .is_err()
        );
        assert!(
            op("pages.count", "/v1/spaces/{space}")
                .validate("pages")
                .is_err()
        );
        let get_mutation = Operation::mutation(
            "pages.x",
            HttpMethod::Get,
            "/v1/x",
            Scope::Read,
            |_c, i: Input| async move { Ok(Output { total: i.count }) },
        );
        assert!(get_mutation.validate("pages").is_err());
    }

    #[test]
    fn params_and_metadata() {
        let op = op("pages.count", "/v1/spaces/{space_id}/docs/{doc_id}")
            .summary("Counts")
            .fresh_auth(Duration::from_mins(5))
            .allow_anonymous()
            .budget(Duration::from_millis(50))
            .deprecated();
        assert_eq!(op.path_params().collect::<Vec<_>>(), ["space_id", "doc_id"]);
        assert_eq!(op.summary_text(), "Counts");
        assert_eq!(op.fresh_auth_window(), Some(Duration::from_mins(5)));
        assert_eq!(op.time_budget(), Duration::from_millis(50));
        assert!(op.is_deprecated());
        assert!(op.allows_anonymous());
        assert!(format!("{op:?}").contains("pages.count"));
    }

    #[tokio::test]
    async fn handler_round_trip() {
        let op = op("pages.count", "/v1/spaces/{space_id}/count");
        let out = op
            .call(ctx(), serde_json::json!({"space_id": "s", "count": 21}))
            .await;
        assert_eq!(out.ok(), Some(serde_json::json!({"total": 42})));
        let bad = op.call(ctx(), serde_json::json!({"space_id": "s"})).await;
        assert_eq!(bad.err().map(|e| e.kind()), Some(ErrorKind::InvalidInput));
    }

    #[test]
    fn remaining_time_saturates() {
        let mut c = ctx();
        c.deadline = Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
        assert_eq!(c.remaining(), Duration::ZERO);
    }
}
