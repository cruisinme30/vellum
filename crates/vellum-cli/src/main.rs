// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The `vellum` binary.
//!
//! One binary runs every Vellum service. `vellum serve` starts the gateway
//! with every service registered in [`services`]; `vellum openapi` and
//! `vellum graphql-schema` print the API contracts the clients are
//! generated from. `vellum docs` reads the help books built into the
//! binary; `vellum mcp` serves the MCP server over stdio; `vellum reindex`
//! rebuilds the index from the data folder. `vellum profile` and
//! `vellum budgets` ask a running server for a flame graph and for routes
//! over their latency budget.

mod admin;
mod docs;
mod identity;
#[cfg(test)]
mod identity_tests;
mod mcp;
mod reindex;
mod serve;
mod services;

use std::io::Write as _;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use vellum_kernel::telemetry::{self, TelemetryConfig};

/// Vellum: a self-hosted workspace where every object is a Markdown file.
#[derive(Debug, Parser)]
#[command(name = "vellum", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

/// The subcommands.
#[derive(Debug, Subcommand)]
enum Command {
    /// Read the user, admin and developer guides built into this binary.
    Docs(docs::DocsArgs),
    /// Serve the MCP server over stdio for a local AI client.
    Mcp(mcp::McpArgs),
    /// Runs the server: every service behind the gateway.
    Serve {
        /// The configuration file (default: `VELLUM_CONFIG`, then built-in
        /// defaults and `VELLUM_*` variables).
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Administration that needs access to the server's files.
    #[command(subcommand)]
    Admin(AdminCommand),
    /// Prints the OpenAPI 3.1 document of every service.
    Openapi,
    /// Prints the `GraphQL` schema of every service.
    GraphqlSchema,
    /// Rebuild the index from the files in the data folder.
    Reindex(reindex::Args),
    /// Draw a flame graph of where a running server spends its time.
    Profile(vellum_admin::profile::ProfileArgs),
    /// Fail when a route's p95 latency is over its budget (a CI gate after
    /// a load test).
    Budgets(vellum_admin::budgets::BudgetsArgs),
}

/// Administration subcommands.
#[derive(Debug, Subcommand)]
enum AdminCommand {
    /// Issues a one-time code for a 15-minute administrator session, for
    /// when no administrator can sign in (SSO down, accounts lost).
    /// Redeem it in the browser with `POST /auth/break-glass`.
    BreakGlass {
        /// The configuration file, to find the data directory.
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
}

fn break_glass(config: Option<&std::path::Path>) -> ExitCode {
    let config = match vellum_kernel::Config::load(config) {
        Ok(c) => c,
        Err(error) => return fail(&error),
    };
    let lifetime = std::time::Duration::from_mins(15);
    match vellum_identity::Identity::issue_break_glass(&config.data_dir, lifetime) {
        Ok(code) => print(&format!(
            "Break-glass code (valid once, for 15 minutes):\n{code}\n\n\
             Redeem it on the sign-in page, or with\n\
             curl -X POST -H 'content-type: application/json' -d '{{\"code\":\"<code>\"}}' <url>/auth/break-glass"
        )),
        Err(error) => fail(&error),
    }
}

fn fail(message: &dyn std::fmt::Display) -> ExitCode {
    // Standard error is the only channel when telemetry is not running.
    #[expect(clippy::print_stderr, reason = "the error must reach the operator")]
    {
        eprintln!("vellum: {message}");
    }
    ExitCode::FAILURE
}

fn print(text: &str) -> ExitCode {
    let mut out = std::io::stdout().lock();
    match out
        .write_all(text.as_bytes())
        .and_then(|()| out.write_all(b"\n"))
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(&error),
    }
}

/// Starts telemetry and reports that nothing was asked for.
fn no_command() -> ExitCode {
    let config = TelemetryConfig::from_env("vellum", env!("CARGO_PKG_VERSION"));
    let _guard = match telemetry::init(&config) {
        Ok(guard) => guard,
        Err(error) => return fail(&error),
    };
    let span = tracing::info_span!("vellum.cli", version = env!("CARGO_PKG_VERSION"));
    let _entered = span.enter();
    tracing::info!("vellum started; no command given");
    ExitCode::SUCCESS
}

/// Runs `f` with telemetry on, inside the `vellum.cli` span.
fn with_telemetry(f: impl FnOnce() -> ExitCode) -> ExitCode {
    let config = TelemetryConfig::from_env("vellum", env!("CARGO_PKG_VERSION"));
    let _guard = match telemetry::init(&config) {
        Ok(guard) => guard,
        Err(error) => return fail(&error),
    };
    let span = tracing::info_span!("vellum.cli", version = env!("CARGO_PKG_VERSION"));
    let _entered = span.enter();
    f()
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        return no_command();
    };
    match command {
        // Reading the embedded books needs no telemetry or configuration.
        Command::Docs(args) => match docs::run(&args, &mut std::io::stdout().lock()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => fail(&format_args!("docs: {error}")),
        },
        Command::Mcp(args) => {
            let telemetry_config = TelemetryConfig::from_env("vellum", env!("CARGO_PKG_VERSION"));
            let _guard = match telemetry::init(&telemetry_config) {
                Ok(guard) => guard,
                Err(error) => return fail(&error),
            };
            let span = tracing::info_span!("vellum.cli", version = env!("CARGO_PKG_VERSION"));
            let _entered = span.enter();
            mcp::run(&args)
        }
        Command::Admin(AdminCommand::BreakGlass { config }) => break_glass(config.as_deref()),
        Command::Reindex(args) => {
            let telemetry_config = TelemetryConfig::from_env("vellum", env!("CARGO_PKG_VERSION"));
            let _guard = match telemetry::init(&telemetry_config) {
                Ok(guard) => guard,
                Err(error) => return fail(&error),
            };
            let span = tracing::info_span!("vellum.cli", version = env!("CARGO_PKG_VERSION"));
            let _entered = span.enter();
            reindex::run(&args)
        }
        Command::Profile(args) => with_telemetry(|| admin::profile(&args)),
        Command::Budgets(args) => with_telemetry(|| admin::budgets(&args)),
        Command::Openapi => match services::contract_gateway() {
            Ok(gateway) => {
                print(&serde_json::to_string_pretty(gateway.openapi()).unwrap_or_default())
            }
            Err(error) => fail(&error),
        },
        Command::GraphqlSchema => match services::contract_gateway() {
            Ok(gateway) => print(&gateway.graphql_sdl()),
            Err(error) => fail(&error),
        },
        Command::Serve { config } => {
            let telemetry_config = TelemetryConfig::from_env("vellum", env!("CARGO_PKG_VERSION"));
            let _guard = match telemetry::init(&telemetry_config) {
                Ok(guard) => guard,
                Err(error) => return fail(&error),
            };
            match serve::run(config.as_deref()) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    tracing::error!(error = %error, "vellum serve failed");
                    fail(&error)
                }
            }
        }
    }
}
