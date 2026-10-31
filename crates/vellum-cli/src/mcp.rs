// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum mcp`: the MCP server over stdio for a local client such as
//! Claude Desktop or Claude Code.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use vellum_mcp::auth::{Principal, ScopeSet};
use vellum_mcp::memory::{MemoryAudit, MemoryWorkspace};
use vellum_mcp::ports::Workspace;
use vellum_mcp::server::{McpServer, ServerConfig};
use vellum_private::{PrivateAgent, PrivateSpace};
use vellum_store::{BlobPath, BlobStore, LocalStore};

use crate::private::{PrivateWorkspace, device_identity, parse_space};
use crate::private_remote::{E2eClient, RemoteSpaces};

/// Options of `vellum mcp`.
#[derive(Debug, clap::Args)]
pub struct McpArgs {
    /// Serve the in-memory Northstar demo workspace instead of this
    /// machine's workspace.
    #[arg(long, conflicts_with = "private")]
    demo: bool,
    /// Serve the end-to-end encrypted private spaces sealed to this device,
    /// read-only. The pages are opened in memory with the device key from
    /// the OS keychain; nothing decrypted is written to disk.
    #[arg(long, requires = "space")]
    private: bool,
    /// A private space as `slug=path`, the path relative to the data folder
    /// (for example `board=drives/shared/board`); with `--server`, as
    /// `slug=<space ID>`. Repeat for more spaces.
    #[arg(long, value_parser = parse_space)]
    space: Vec<(String, String)>,
    /// How often to reopen the private spaces, in seconds, to pick up
    /// pages synced since the last pass.
    #[arg(
        long,
        default_value_t = 60,
        value_parser = clap::value_parser!(u64).range(5..)
    )]
    refresh_secs: u64,
    /// Read the private spaces from this Vellum server instead of the data
    /// folder. Only ciphertext is downloaded; the token comes from
    /// `VELLUM_TOKEN`.
    #[arg(long, requires = "private")]
    server: Option<String>,
}

/// Runs the stdio server until standard input closes.
pub fn run(args: &McpArgs) -> ExitCode {
    if !args.demo && !args.private {
        tracing::error!(
            "`vellum mcp` serves the demo (`--demo`) or this device's private \
                spaces (`--private --space slug=path`); for the workspace, \
                point your MCP client at the server's /mcp endpoint, such as \
                https://vellum.example.com/mcp"
        );
        return ExitCode::FAILURE;
    }
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
    let result = runtime.block_on(async {
        if args.private {
            let agent =
                open_private(&args.space, args.server.as_deref()).await?;
            keep_fresh(
                Arc::clone(&agent),
                Duration::from_secs(args.refresh_secs),
            );
            // Reads only: write tools are not offered at all.
            serve(
                Arc::new(PrivateWorkspace::new(agent)),
                principal("private", "local", "read"),
            )
            .await
        } else {
            let scopes = std::env::var("VELLUM_SCOPES")
                .unwrap_or_else(|_| "read write:draft work".to_owned());
            serve(
                Arc::new(MemoryWorkspace::northstar()),
                principal("demo", "usr_priya", &scopes),
            )
            .await
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "vellum mcp failed");
            crate::fail(&error)
        }
    }
}

fn principal(token: &str, user: &str, scopes: &str) -> Principal {
    Principal {
        token_id: token.into(),
        user_id: user.into(),
        user_name: user.into(),
        client_id: "stdio".into(),
        client_name: "Local MCP client".into(),
        scopes: ScopeSet::parse(scopes),
    }
}

/// Opens the private spaces with this device's key: from the data folder,
/// or from `server` through its `e2e` API.
async fn open_private(
    spaces: &[(String, String)],
    server: Option<&str>,
) -> Result<Arc<PrivateAgent>, String> {
    let identity = device_identity()?;
    let store: Arc<dyn BlobStore> = if let Some(server) = server {
        let token =
            std::env::var(crate::private_remote::TOKEN_VAR).unwrap_or_default();
        Arc::new(RemoteSpaces::new(E2eClient::new(server, &token)?))
    } else {
        let config = vellum_kernel::config::Config::load(None)
            .map_err(|e| e.to_string())?;
        Arc::new(LocalStore::open(&config.data_dir).await.map_err(|e| {
            format!("cannot open {}: {e}", config.data_dir.display())
        })?)
    };
    let mut opened = Vec::with_capacity(spaces.len());
    for (slug, path) in spaces {
        let root =
            BlobPath::new(path.as_str()).map_err(|e| format!("{slug}: {e}"))?;
        opened.push(
            PrivateSpace::open(store.as_ref(), &identity, slug, root)
                .await
                .map_err(|e| format!("{slug}: {e}"))?,
        );
    }
    tracing::info!(
        fingerprint = %identity.fingerprint_hex(),
        spaces = opened.len(),
        "opening private spaces"
    );
    let agent = PrivateAgent::new(identity, store, opened)
        .map_err(|e| e.to_string())?;
    let report = agent.refresh().await.map_err(|e| e.to_string())?;
    tracing::info!(
        opened = report.opened,
        not_for_this_device = report.not_for_this_device,
        unreadable = report.unreadable,
        not_sealed = report.not_sealed,
        "private spaces ready"
    );
    Ok(Arc::new(agent))
}

/// Reopens the spaces every `every`, so pages synced in since start show up.
fn keep_fresh(agent: Arc<PrivateAgent>, every: Duration) {
    drop(tokio::spawn(async move {
        let mut tick = tokio::time::interval(every);
        // The first tick is immediate, and the spaces were just opened.
        let _ = tick.tick().await;
        loop {
            let _ = tick.tick().await;
            if let Err(error) = agent.refresh().await {
                tracing::warn!(%error, "could not reopen the private spaces");
            }
        }
    }));
}

async fn serve(
    workspace: Arc<dyn Workspace>,
    principal: Principal,
) -> Result<(), String> {
    let server = Arc::new(McpServer::new(
        workspace,
        Arc::new(MemoryAudit::default()),
        ServerConfig::default(),
    ));
    tracing::info!(scopes = %principal.scopes, "serving MCP over stdio");
    vellum_mcp::stdio::serve(
        server,
        principal,
        tokio::io::stdin(),
        tokio::io::stdout(),
    )
    .await
    .map_err(|e| format!("the MCP stdio transport failed: {e}"))
}
