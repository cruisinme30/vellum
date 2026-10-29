// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum mcp`: the MCP server over stdio for a local client such as
//! Claude Desktop or Claude Code.

use std::process::ExitCode;
use std::sync::Arc;

use vellum_mcp::auth::{Principal, ScopeSet};
use vellum_mcp::memory::{MemoryAudit, MemoryWorkspace};
use vellum_mcp::server::{McpServer, ServerConfig};

/// Options of `vellum mcp`.
#[derive(Debug, clap::Args)]
pub struct McpArgs {
    /// Serve the in-memory Northstar demo workspace instead of this
    /// machine's workspace.
    #[arg(long)]
    demo: bool,
}

/// Runs the stdio server until standard input closes.
pub fn run(args: &McpArgs) -> ExitCode {
    if !args.demo {
        tracing::error!(
            "the workspace services are not wired into the binary yet; run `vellum mcp --demo` to try the server"
        );
        return ExitCode::FAILURE;
    }
    let scopes =
        std::env::var("VELLUM_SCOPES").unwrap_or_else(|_| "read write:draft work".to_owned());
    let principal = Principal {
        token_id: "demo".into(),
        user_id: "usr_priya".into(),
        user_name: "Priya".into(),
        client_id: "stdio".into(),
        client_name: "Local MCP client".into(),
        scopes: ScopeSet::parse(&scopes),
    };
    let server = Arc::new(McpServer::new(
        Arc::new(MemoryWorkspace::northstar()),
        Arc::new(MemoryAudit::default()),
        ServerConfig::default(),
    ));
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(error) => {
            tracing::error!(%error, "cannot start the async runtime");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(scopes = %principal.scopes, "serving MCP over stdio");
    let result = runtime.block_on(vellum_mcp::stdio::serve(
        server,
        principal,
        tokio::io::stdin(),
        tokio::io::stdout(),
    ));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the MCP stdio transport failed");
            ExitCode::FAILURE
        }
    }
}
