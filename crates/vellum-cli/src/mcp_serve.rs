// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! MCP inside `vellum serve` (ADR 0019, D217–D219).
//!
//! `/mcp` is an `OAuth` 2.1 resource server: tokens are checked by the
//! identity service ([`IdentityTokens`]), calls run against the real
//! workspace ([`crate::mcp_workspace::ServeWorkspace`]), every call is
//! written to the admin audit log ([`AuditTrail`]), the per-token rate
//! limit is shared by every replica, and `subscriptions/listen` streams
//! changes from the cluster's change feed, so a listener on one replica
//! hears about writes made on any of them ([`Events`]).

use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use vellum_admin::{AuditActor, AuditLog, NewAuditEvent, Outcome};
use vellum_db::feed::{Change, PATH, SPACES};
use vellum_drive::{DriveIndex, NodeKind};
use vellum_identity::Identity;
use vellum_kernel::{ErrorKind, SpaceId};
use vellum_mcp::auth::{ScopeSet, TokenError, TokenVerifier, VerifiedToken};
use vellum_mcp::http::{HttpConfig, McpHttp};
use vellum_mcp::ports::{AuditEvent, AuditSink, WorkspaceError};
use vellum_mcp::server::{McpServer, Notifier, ServerConfig};
use vellum_store::BlobPath;
use vellum_store::layout::Kind;

/// Checks bearer tokens with the identity service (D217).
///
/// `OAuth` access tokens must name this server's `/mcp` as their audience
/// (RFC 8707), so a token minted for another resource is refused. Personal
/// API tokens carry no audience: they are only ever valid on the server
/// that issued them, so they are accepted here as if issued for `/mcp`.
#[derive(Debug)]
pub struct IdentityTokens {
    identity: Arc<Identity>,
    resource: String,
}

impl IdentityTokens {
    /// Verifies tokens for the resource `resource` (`<base>/mcp`).
    #[must_use]
    pub const fn new(identity: Arc<Identity>, resource: String) -> Self {
        Self { identity, resource }
    }
}

fn system_time(t: vellum_kernel::Timestamp) -> SystemTime {
    let ms = u64::try_from(t.unix_ms()).unwrap_or_default();
    SystemTime::UNIX_EPOCH + Duration::from_millis(ms)
}

#[async_trait]
impl TokenVerifier for IdentityTokens {
    async fn verify(&self, token: &str) -> Result<VerifiedToken, TokenError> {
        let info =
            self.identity
                .verify_token(token)
                .map_err(|e| match e.kind() {
                    ErrorKind::Unavailable
                    | ErrorKind::Internal
                    | ErrorKind::Timeout => {
                        TokenError::Unavailable(e.message().to_owned())
                    }
                    _ => TokenError::Invalid,
                })?;
        let audience = if info.audience.is_empty() {
            vec![self.resource.clone()]
        } else {
            info.audience
        };
        let scopes: Vec<&str> = info
            .scopes
            .iter()
            .map(vellum_contracts::Scope::as_str)
            .collect();
        Ok(VerifiedToken {
            token_id: info.token_id,
            user_id: info.user.to_string(),
            user_name: info.user_name,
            client_id: info.client_id,
            client_name: info.client_name,
            scopes: ScopeSet::parse(&scopes.join(" ")),
            audience,
            expires_at: info.expires_at.map(system_time),
        })
    }
}

/// Writes MCP calls to the admin audit log once the admin console is
/// open, and to the server log until then (or without one).
#[derive(Default)]
pub struct AuditTrail {
    log: OnceLock<Arc<dyn AuditLog>>,
}

impl std::fmt::Debug for AuditTrail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditTrail")
            .field("log", &self.log.get().is_some())
            .finish()
    }
}

impl AuditTrail {
    /// Starts appending to `log`.
    pub fn attach(&self, log: Arc<dyn AuditLog>) {
        let _ = self.log.set(log);
    }
}

/// The audit record of one MCP call: who, through which client, what.
fn audit_record(event: &AuditEvent) -> NewAuditEvent {
    let outcome = match event.outcome.as_str() {
        "ok" => Outcome::Success,
        "denied" | "rate_limited" => Outcome::Denied,
        _ => Outcome::Failure,
    };
    let actor = AuditActor {
        kind: "model".to_owned(),
        user_id: (event.user_id != "anonymous").then(|| event.user_id.clone()),
        via: Some(format!("{} ({})", event.client_id, event.token_id)),
    };
    let mut record = NewAuditEvent::new(actor, format!("mcp.{}", event.method))
        .outcome(outcome)
        .detail("duration_ms", event.duration_ms)
        .detail("result", event.outcome.clone());
    if let Some(target) = &event.target {
        record = record.target(target.clone());
    }
    if !event.touched.is_empty() {
        record = record.detail("touched", event.touched.clone());
    }
    record.request_id = Some(event.request_id.clone());
    record.trace_id.clone_from(&event.trace_id);
    record
}

#[async_trait]
impl AuditSink for AuditTrail {
    async fn record(&self, event: AuditEvent) -> Result<(), WorkspaceError> {
        let Some(log) = self.log.get() else {
            tracing::info!(
                target: "vellum::audit",
                method = %event.method,
                user = %event.user_id,
                client = %event.client_id,
                outcome = %event.outcome,
                "mcp call"
            );
            return Ok(());
        };
        log.append(audit_record(&event))
            .await
            .map(|_| ())
            .map_err(|e| WorkspaceError::Unavailable(e.to_string()))
    }
}

/// Turns the cluster's change feed into `subscriptions/listen` events
/// (D219). Every replica follows the whole feed, its own writes included,
/// so a listener hears about a change wherever it was made.
#[derive(Clone)]
pub struct Events {
    notifier: Arc<OnceLock<Notifier>>,
    index: Arc<dyn DriveIndex>,
    space_of: vellum_collections::service::SpaceOf,
}

impl std::fmt::Debug for Events {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Events")
            .field("attached", &self.notifier.get().is_some())
            .finish_non_exhaustive()
    }
}

/// What one change means to MCP listeners.
#[derive(Debug, PartialEq, Eq)]
enum Meaning {
    /// The list of spaces, books or collections may have changed.
    List,
    /// An issue changed.
    Issue(String),
    /// A document file changed; its ID comes from the drive index.
    Document(BlobPath),
    /// Nothing a listener can subscribe to.
    Nothing,
}

fn meaning(change: &Change) -> Meaning {
    match change.topic.as_str() {
        SPACES => Meaning::List,
        PATH => {
            let Ok(path) = BlobPath::new(&change.subject) else {
                return Meaning::Nothing;
            };
            match Kind::of(&path) {
                Kind::BookManifest { .. } | Kind::CollectionSchema { .. } => {
                    Meaning::List
                }
                Kind::Issue { team, number } => {
                    Meaning::Issue(format!("{team}-{number}"))
                }
                Kind::Document => Meaning::Document(path),
                _ => Meaning::Nothing,
            }
        }
        _ => Meaning::Nothing,
    }
}

impl Events {
    /// Events for documents found through `index` and `space_of`.
    #[must_use]
    pub fn new(
        index: Arc<dyn DriveIndex>,
        space_of: vellum_collections::service::SpaceOf,
    ) -> Self {
        Self {
            notifier: Arc::new(OnceLock::new()),
            index,
            space_of,
        }
    }

    /// Starts delivering to `notifier`'s listeners.
    pub fn attach(&self, notifier: Notifier) {
        let _ = self.notifier.set(notifier);
    }

    /// Publishes a page of the feed.
    pub fn changed(&self, changes: &[Change]) {
        let Some(notifier) = self.notifier.get().cloned() else {
            return;
        };
        let meanings: Vec<Meaning> = changes.iter().map(meaning).collect();
        if meanings.iter().all(|m| *m == Meaning::Nothing) {
            return;
        }
        let me = self.clone();
        drop(tokio::spawn(async move {
            let mut list = false;
            for m in meanings {
                match m {
                    Meaning::List => list = true,
                    Meaning::Issue(key) => {
                        notifier
                            .resource_updated(format!("vellum://issue/{key}"));
                    }
                    Meaning::Document(path) => {
                        if let Some(uri) = me.document(&path).await {
                            notifier.resource_updated(uri);
                        }
                    }
                    Meaning::Nothing => {}
                }
            }
            if list {
                notifier.resource_list_changed();
            }
        }));
    }

    /// Missed changes: every listener should refetch.
    pub fn reset(&self) {
        if let Some(notifier) = self.notifier.get() {
            notifier.resource_list_changed();
        }
    }

    /// The deepest space whose folder holds `path`.
    async fn space_at(&self, path: &BlobPath) -> Option<SpaceId> {
        self.index
            .spaces()
            .await
            .ok()?
            .into_iter()
            .filter(|s| {
                path.as_str()
                    .strip_prefix(s.root.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|s| s.root.as_str().len())
            .map(|s| s.id)
    }

    async fn document(&self, path: &BlobPath) -> Option<String> {
        let space = match (self.space_of)(path) {
            Some(id) => SpaceId::from_u128(id.0),
            // A space made since the lookup was last loaded.
            None => self.space_at(path).await?,
        };
        let node = self.index.node_at(space, path).await.ok().flatten()?;
        let kind = match node.kind {
            NodeKind::Page => "doc",
            NodeKind::Book => "book",
            NodeKind::Collection => "collection",
            _ => return None,
        };
        Some(format!("vellum://{kind}/{}", node.id))
    }
}

/// Builds the `/mcp` endpoint over `workspace` for the deployment at
/// `base_url`, with tokens from `identity` and the per-token limit shared
/// through `shared` when the server has a cluster table.
#[must_use]
pub fn endpoint(
    workspace: Arc<dyn vellum_mcp::ports::Workspace>,
    identity: &Arc<Identity>,
    base_url: &str,
    audit: Arc<AuditTrail>,
    shared: Option<Arc<dyn vellum_mcp::ratelimit::SharedLimit>>,
) -> Arc<McpHttp> {
    let config = HttpConfig::new(base_url);
    let mut server = McpServer::new(workspace, audit, ServerConfig::default());
    if let Some(shared) = shared {
        server = server.with_shared_limit(shared);
    }
    Arc::new(McpHttp {
        server: Arc::new(server),
        verifier: Arc::new(IdentityTokens::new(
            Arc::clone(identity),
            config.resource.resource(),
        )),
        config,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(topic: &str, subject: &str) -> Change {
        Change {
            seq: 1,
            topic: topic.to_owned(),
            subject: subject.to_owned(),
            origin: "a".to_owned(),
        }
    }

    #[test]
    fn feed_changes_map_to_resources() {
        assert_eq!(meaning(&change(SPACES, "")), Meaning::List);
        assert_eq!(
            meaning(&change(PATH, "teams/ENG/issues/ENG-142.md")),
            Meaning::Issue("ENG-142".into())
        );
        assert!(matches!(
            meaning(&change(PATH, "spaces/eng/launch.md")),
            Meaning::Document(_)
        ));
        assert_eq!(
            meaning(&change(PATH, ".vellum/state.json")),
            Meaning::Nothing
        );
        assert_eq!(meaning(&change("future-topic", "x")), Meaning::Nothing);
    }

    #[test]
    fn audit_records_name_the_model_and_its_client() {
        let record = audit_record(&AuditEvent {
            at: "2026-10-29T21:00:00Z".into(),
            request_id: "req_1".into(),
            trace_id: None,
            user_id: "u1".into(),
            client_id: "https://claude.ai/client.json".into(),
            token_id: "tok_1".into(),
            method: "tools/call".into(),
            target: Some("create_draft".into()),
            touched: vec!["01JD".into()],
            outcome: "ok".into(),
            duration_ms: 12,
        });
        assert_eq!(record.action, "mcp.tools/call");
        assert_eq!(record.actor.kind, "model");
        assert_eq!(record.actor.user_id.as_deref(), Some("u1"));
        assert_eq!(record.target.as_deref(), Some("create_draft"));
        assert_eq!(record.outcome, Outcome::Success);
        assert_eq!(record.request_id.as_deref(), Some("req_1"));
    }
}
