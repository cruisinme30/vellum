// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Bring-your-own-model features in `vellum serve` (ADR 0019, D238-D240).
//!
//! The assistant reads and writes through the same workspace adapter as
//! the MCP server, acting as the signed-in user with a read (and, when the
//! session allows it, draft) scope, so it sees exactly what that user's
//! MCP client would.
//!
//! | Variable | Meaning |
//! | --- | --- |
//! | `VELLUM_AI` | `off` removes the assist API |
//! | `VELLUM_AI_ENDPOINTS` | Comma-separated base URLs of self-hosted models users may pick (http and private addresses allowed) |

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use vellum_assist::sources::{Created, DraftPage, Hit, Page};
use vellum_assist::{AssistService, HttpModel, Sources};
use vellum_contracts::{Principal, Scope, Service};
use vellum_kernel::{Error, ErrorKind};
use vellum_mcp::auth::{
    Principal as McpPrincipal, Scope as McpScope, ScopeSet as McpScopes,
};
use vellum_mcp::ports::{
    Actor, Attribution, NewPage, SearchQuery, Workspace, WorkspaceError,
    WriteMode,
};

/// The client name the MCP layer records for assistant calls.
const CLIENT: &str = "vellum-assist";

/// How long one model call may take.
const MODEL_TIMEOUT: Duration = Duration::from_mins(3);

/// The [`Sources`] port over the MCP workspace adapter.
pub struct McpSources {
    workspace: Arc<dyn Workspace>,
}

impl McpSources {
    /// Wraps the server's MCP workspace adapter.
    pub fn new(workspace: Arc<dyn Workspace>) -> Self {
        Self { workspace }
    }
}

fn actor(who: &Principal) -> Result<Actor, Error> {
    let user = who.require_user()?.to_string();
    let scopes = [Scope::Read, Scope::WriteDraft]
        .into_iter()
        .filter(|s| who.scopes().allows(*s))
        .map(|s| match s {
            Scope::WriteDraft => McpScope::WriteDraft,
            _ => McpScope::Read,
        });
    Ok(Actor::User(McpPrincipal {
        token_id: format!("{CLIENT}:{user}"),
        user_id: user.clone(),
        user_name: user,
        client_id: CLIENT.to_owned(),
        client_name: "Vellum assistant (your own model)".to_owned(),
        scopes: McpScopes::new(scopes),
    }))
}

fn error(e: &WorkspaceError) -> Error {
    let kind = match e {
        WorkspaceError::NotFound(_) => ErrorKind::NotFound,
        WorkspaceError::Forbidden(_) => ErrorKind::PermissionDenied,
        WorkspaceError::Invalid(_) => ErrorKind::InvalidInput,
        WorkspaceError::Conflict(_) => ErrorKind::Conflict,
        WorkspaceError::Unavailable(_) => ErrorKind::Unavailable,
    };
    Error::new(kind, e.to_string())
}

#[async_trait]
impl Sources for McpSources {
    async fn search(
        &self,
        who: &Principal,
        query: &str,
        space: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Hit>, Error> {
        let hits = self
            .workspace
            .search(
                &actor(who)?,
                SearchQuery {
                    query: query.to_owned(),
                    space: space.map(str::to_owned),
                    limit,
                    ..SearchQuery::default()
                },
            )
            .await
            .map_err(|e| error(&e))?;
        Ok(hits
            .into_iter()
            .map(|h| Hit {
                doc_id: h.doc_id,
                title: h.title,
                heading_path: h.heading_path,
                snippet: h.snippet,
                url: h.citation_url,
            })
            .collect())
    }

    async fn page(&self, who: &Principal, id: &str) -> Result<Page, Error> {
        let doc = self
            .workspace
            .get_doc(&actor(who)?, id)
            .await
            .map_err(|e| error(&e))?;
        Ok(Page {
            id: doc.id,
            title: doc.title,
            markdown: doc.markdown,
            url: doc.url,
            space: doc.space,
        })
    }

    async fn create_draft(
        &self,
        who: &Principal,
        page: DraftPage,
    ) -> Result<Created, Error> {
        let user = who.require_user()?.to_string();
        let receipt = self
            .workspace
            .create_page(
                &actor(who)?,
                NewPage {
                    parent: page.parent,
                    title: page.title,
                    markdown: page.markdown,
                    template: page.template,
                },
                WriteMode::Draft,
                Attribution {
                    authored_by: "model".to_owned(),
                    on_behalf_of: user,
                    client: CLIENT.to_owned(),
                },
            )
            .await
            .map_err(|e| error(&e))?;
        Ok(Created {
            id: receipt.id,
            url: receipt.url,
        })
    }
}

/// The assist service over `workspace`, or `None` when `VELLUM_AI=off`.
///
/// # Errors
///
/// A malformed `VELLUM_AI_ENDPOINTS`.
pub fn service(
    workspace: Arc<dyn Workspace>,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Option<Arc<dyn Service>>, Error> {
    if var("VELLUM_AI").is_some_and(|v| v.trim().eq_ignore_ascii_case("off")) {
        tracing::info!("VELLUM_AI=off: the bring-your-own-model API is off");
        return Ok(None);
    }
    let endpoints: Vec<String> = var("VELLUM_AI_ENDPOINTS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(str::to_owned)
        .collect();
    let model = HttpModel::new(&endpoints, MODEL_TIMEOUT)?;
    Ok(Some(Arc::new(AssistService::new(
        Arc::new(model),
        Arc::new(McpSources::new(workspace)),
    ))))
}

#[cfg(test)]
mod tests {
    use vellum_contracts::{CredentialKind, ScopeSet};
    use vellum_kernel::{Timestamp, UserId};
    use vellum_mcp::memory::MemoryWorkspace;

    use super::*;

    fn who(scopes: &[Scope]) -> Principal {
        Principal::user(
            UserId::generate(),
            CredentialKind::Session,
            ScopeSet::from_scopes(scopes.iter().copied()),
            Timestamp::now(),
        )
    }

    #[tokio::test]
    async fn reads_as_the_user_and_drafts_for_review() -> Result<(), Error> {
        let memory = Arc::new(MemoryWorkspace::northstar());
        let sources = McpSources::new(memory.clone());
        let reader = who(&[Scope::Read]);
        let hits = sources.search(&reader, "policy simulator", None, 5).await?;
        assert!(!hits.is_empty(), "the demo launch plan mentions it");
        let page = sources.page(&reader, &hits[0].doc_id).await?;
        assert_eq!(page.id, hits[0].doc_id);
        let created = sources
            .create_draft(
                &who(&[Scope::Read, Scope::WriteDraft]),
                DraftPage {
                    parent: format!("space:{}", page.space),
                    title: "Draft".into(),
                    markdown: "Body".into(),
                    template: Some(page.id.clone()),
                },
            )
            .await?;
        assert!(!created.id.is_empty());
        let writes = memory.writes();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].mode, WriteMode::Draft);
        Ok(())
    }

    #[test]
    fn off_switch_and_endpoint_check() -> Result<(), Error> {
        let memory: Arc<dyn Workspace> = Arc::new(MemoryWorkspace::northstar());
        let off = service(Arc::clone(&memory), |k| {
            (k == "VELLUM_AI").then(|| "off".to_owned())
        })?;
        assert!(off.is_none());
        let on = service(Arc::clone(&memory), |k| {
            (k == "VELLUM_AI_ENDPOINTS")
                .then(|| "http://ollama:11434/v1".to_owned())
        })?;
        assert_eq!(on.map(|s| s.name()), Some("assist"));
        let bad = service(memory, |k| {
            (k == "VELLUM_AI_ENDPOINTS").then(|| "not a url".to_owned())
        });
        assert!(bad.is_err());
        Ok(())
    }
}
