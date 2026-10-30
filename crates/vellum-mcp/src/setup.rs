// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! One-click setup: configuration snippets for MCP clients and the
//! "Test connection" check behind the Admin page button.
//!
//! Snippets never contain a secret; they refer to `VELLUM_TOKEN`, which the
//! user pastes from the token they create for the client.

use std::sync::Arc;

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::{Value, json};

use crate::auth::Scope;
use crate::http::{McpHttp, request_body};
use crate::protocol::Incoming;
use crate::server::{Reply, RequestCtx};

/// One client's setup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snippet {
    /// Client id: `claude-desktop`, `claude-code`, `cursor`, `generic`.
    pub client: &'static str,
    /// Display name.
    pub title: &'static str,
    /// Where the snippet goes.
    pub instructions: String,
    /// `json` or `shell`.
    pub language: &'static str,
    /// The snippet.
    pub code: String,
}

/// The snippets for a deployment at `base_url`.
#[must_use]
pub fn snippets(base_url: &str) -> Vec<Snippet> {
    let mcp = format!("{}/mcp", base_url.trim_end_matches('/'));
    let stdio = json!({ "mcpServers": { "vellum": {
        "command": "vellum", "args": [
            "mcp"
        ], "env": { "VELLUM_TOKEN": "<paste your token>" }
    } } });
    vec![
        Snippet {
            client: "claude-desktop",
            title: "Claude Desktop",
            instructions: format!(
                "Settings → Connectors → Add custom \
                connector, and enter {mcp}; Claude signs you in with OAuth. \
                When Vellum runs on this computer, you can use stdio instead: \
                add this to claude_desktop_config.json:"
            ),
            language: "json",
            code: serde_json::to_string_pretty(&stdio).unwrap_or_default(),
        },
        Snippet {
            client: "claude-code",
            title: "Claude Code",
            instructions:
                "Run in a terminal. Claude Code opens the browser to \
                sign in the first time."
                    .into(),
            language: "shell",
            code: format!("claude mcp add --transport http vellum {mcp}"),
        },
        Snippet {
            client: "cursor",
            title: "Cursor",
            instructions:
                "Add to ~/.cursor/mcp.json (or .cursor/mcp.json in a \
                project). Cursor signs you in with OAuth."
                    .into(),
            language: "json",
            code: serde_json::to_string_pretty(
                &json!({ "mcpServers": { "vellum": { "url": mcp } } }),
            )
            .unwrap_or_default(),
        },
        Snippet {
            client: "generic",
            title: "Any MCP client",
            instructions: "Streamable HTTP, protocol 2026-07-28 (2025-11-25 \
                also accepted). OAuth 2.1: the server's Protected Resource \
                Metadata names the authorization server. On the machine that \
                runs Vellum, `vellum mcp` serves stdio with VELLUM_TOKEN set \
                to an API token."
                .into(),
            language: "json",
            code: serde_json::to_string_pretty(&json!({
                "url": mcp,
                "transport": "streamable-http",
                "protectedResourceMetadata": format!("{}/.well-known/oauth-prot\
                    ected-resource/mcp", base_url.trim_end_matches('/'))
            }))
            .unwrap_or_default(),
        },
    ]
}

/// One step of the connection check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// Step name.
    pub name: &'static str,
    /// Whether it passed.
    pub ok: bool,
    /// What happened, in a sentence.
    pub detail: String,
}

/// The result of "Test connection".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectionReport {
    /// Every step passed.
    pub ok: bool,
    /// Steps in order; stops at the first failure.
    pub checks: Vec<Check>,
    /// The token's effective scopes.
    pub scopes: Vec<String>,
}

fn check(name: &'static str, ok: bool, detail: impl Into<String>) -> Check {
    Check {
        name,
        ok,
        detail: detail.into(),
    }
}

async fn rpc(
    state: &McpHttp,
    ctx: &RequestCtx,
    id: i64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let msg = Incoming::from_value(request_body(id, method, params))
        .map_err(|(_, e)| e.message)?;
    match state.server.handle(msg, ctx).await {
        Reply::Result { result, .. } => Ok(result),
        Reply::Error { error, .. } => Err(error.message),
        Reply::Accepted | Reply::Listen(_) => Err("unexpected reply".into()),
    }
}

/// Runs the connection check for a bearer token, through the same code
/// path a client uses.
#[expect(
    clippy::too_many_lines,
    reason = "the checks run in sequence and stop at the first failure"
)]
pub async fn test_connection(
    state: &McpHttp,
    headers: &HeaderMap,
) -> ConnectionReport {
    let mut checks = Vec::new();
    let principal = match state.authenticate(headers).await {
        Ok(p) => {
            checks.push(check(
                "token",
                true,
                format!(
                    "Signed in as {} through {}.",
                    p.user_name, p.client_name
                ),
            ));
            p
        }
        Err(e) => {
            let detail = e.map_or_else(
                || "No token was sent.".to_owned(),
                |e| {
                    format!(
                        "{e}. Create a new token in Settings → Connected apps."
                    )
                },
            );
            checks.push(check("token", false, detail));
            return ConnectionReport {
                ok: false,
                checks,
                scopes: Vec::new(),
            };
        }
    };
    let scopes: Vec<String> = principal
        .scopes
        .iter()
        .map(|s| s.as_str().to_owned())
        .collect();
    let ctx = RequestCtx {
        principal: principal.clone(),
        request_id: crate::http::request_id(headers),
        fallback_version: None,
    };
    match rpc(state, &ctx, 1, "server/discover", json!({})).await {
        Ok(r) => checks.push(check(
            "discover",
            true,
            format!(
                "The server speaks {}.",
                r.get("supportedVersions")
                    .and_then(Value::as_array)
                    .map(|v| v
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", "))
                    .unwrap_or_default()
            ),
        )),
        Err(e) => {
            checks.push(check("discover", false, e));
            return ConnectionReport {
                ok: false,
                checks,
                scopes,
            };
        }
    }
    match rpc(state, &ctx, 2, "tools/list", json!({})).await {
        Ok(r) => checks.push(check(
            "tools",
            true,
            format!(
                "{} tools available.",
                r.get("tools").and_then(Value::as_array).map_or(0, Vec::len)
            ),
        )),
        Err(e) => {
            checks.push(check("tools", false, e));
            return ConnectionReport {
                ok: false,
                checks,
                scopes,
            };
        }
    }
    if principal.allows(Scope::Read) {
        match rpc(state, &ctx, 3, "resources/list", json!({})).await {
            Ok(r) => checks.push(check(
                "read",
                true,
                format!(
                    "{} readable spaces, books and pages on the first page.",
                    r.get("resources")
                        .and_then(Value::as_array)
                        .map_or(0, Vec::len)
                ),
            )),
            Err(e) => {
                checks.push(check("read", false, e));
                return ConnectionReport {
                    ok: false,
                    checks,
                    scopes,
                };
            }
        }
    } else {
        checks.push(check(
            "read",
            false,
            "The token lacks the read scope, so the client cannot search or \
                read.",
        ));
        return ConnectionReport {
            ok: false,
            checks,
            scopes,
        };
    }
    ConnectionReport {
        ok: true,
        checks,
        scopes,
    }
}

pub(crate) async fn snippets_handler(
    State(state): State<Arc<McpHttp>>,
) -> Response {
    axum::Json(json!({ "snippets": snippets(&state.config.resource.base_url) }))
        .into_response()
}

pub(crate) async fn test_connection_handler(
    State(state): State<Arc<McpHttp>>,
    headers: HeaderMap,
) -> Response {
    let report = test_connection(&state, &headers).await;
    let mut r = axum::Json(report).into_response();
    let _ = r.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_cover_every_client_without_secrets() {
        let s = snippets("https://vellum.northstar.dev/");
        let clients: Vec<&str> = s.iter().map(|s| s.client).collect();
        assert_eq!(
            clients,
            ["claude-desktop", "claude-code", "cursor", "generic"]
        );
        assert!(s.iter().all(|s| !s.code.contains("Bearer")));
        assert!(
            s.iter()
                .any(|s| s.code.contains("https://vellum.northstar.dev/mcp"))
        );
    }
}
