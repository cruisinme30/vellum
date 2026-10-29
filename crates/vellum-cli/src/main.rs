// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The `vellum` binary.
//!
//! One binary runs every Vellum service. `vellum serve` starts the gateway
//! with every service registered in [`services`]; `vellum openapi` and
//! `vellum graphql-schema` print the API contracts the clients are
//! generated from. `vellum docs` reads the help books built into the
//! binary; `vellum mcp` serves the MCP server over stdio; `vellum reindex`
//! rebuilds the index from the data folder; `vellum private` manages this
//! device's key for end-to-end encrypted private spaces. `vellum profile`
//! and `vellum budgets` ask a running server for a flame graph and for
//! routes over their latency budget. `vellum admin` issues break-glass
//! codes, takes, lists and restores encrypted backups, and moves a space
//! to another shard.

mod admin;
mod backup_cli;
mod cluster;
mod collab;
#[cfg(test)]
mod delivery_tests;
mod docs;
mod e2e;
mod feed;
mod google_http;
#[cfg(test)]
mod google_http_tests;
mod help_http;
mod identity;
#[cfg(test)]
mod identity_tests;
mod integrations;
mod keys;
mod limits;
mod mail;
mod mcp;
mod mcp_edit;
mod mcp_serve;
#[cfg(test)]
mod mcp_tests;
mod mcp_workspace;
mod notifications;
mod observability;
mod page_render;
#[cfg(test)]
mod perf_routes_tests;
mod pins;
mod private;
mod private_remote;
#[cfg(test)]
mod private_remote_tests;
#[cfg(test)]
mod private_tests;
mod published;
mod reindex;
mod reminders;
mod search;
mod serve;
mod services;
mod shards_cli;
mod site_http;
#[cfg(test)]
mod site_tests;
#[cfg(test)]
mod sso_tests;
mod task_prefs;
mod tasks;
mod tasks_http;
#[cfg(test)]
mod tasks_tests;
#[cfg(test)]
mod typescript_tests;
mod watcher;
mod web;
#[cfg(test)]
mod web_tests;
mod workspace;

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
    Openapi {
        /// Print the TypeScript client types instead (`web/src/api/api.gen.ts`).
        #[arg(long)]
        typescript: bool,
    },
    /// Prints the `GraphQL` schema of every service.
    GraphqlSchema,
    /// Rebuild the index from the files in the data folder.
    Reindex(reindex::Args),
    /// End-to-end encrypted private spaces on this device.
    #[command(subcommand)]
    Private(private::PrivateCommand),
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
    /// Takes an encrypted backup of the data folder now, into the
    /// destination set in the admin console, and prunes old ones.
    Backup {
        /// The configuration file, to find the data directory.
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Lists the backups in the configured destination.
    Backups {
        /// The configuration file, to find the data directory.
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Restores a backup into an empty folder and checks every file.
    Restore(backup_cli::RestoreArgs),
    /// Moves a space and everything stored about it to another shard
    /// (`VELLUM_SHARDS`). Stop every replica first.
    MoveSpace {
        /// The space's ID.
        space: String,
        /// The shard to move it to.
        #[arg(long)]
        to: String,
    },
}

/// Runs an async admin command on a fresh runtime and prints its lines.
fn run_admin<F>(f: impl FnOnce() -> F) -> ExitCode
where
    F: std::future::Future<Output = Result<Vec<String>, vellum_kernel::Error>>,
{
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return fail(&error),
    };
    match runtime.block_on(f()) {
        Ok(lines) => print(&lines.join("\n")),
        Err(error) => fail(&error),
    }
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
        Command::Admin(AdminCommand::Backup { config }) => {
            with_telemetry(|| run_admin(|| backup_cli::backup_now(config.as_deref())))
        }
        Command::Admin(AdminCommand::Backups { config }) => {
            with_telemetry(|| run_admin(|| backup_cli::list(config.as_deref())))
        }
        Command::Admin(AdminCommand::Restore(args)) => {
            with_telemetry(|| run_admin(|| backup_cli::restore(&args)))
        }
        Command::Admin(AdminCommand::MoveSpace { space, to }) => {
            with_telemetry(|| run_admin(|| shards_cli::move_space(&space, &to)))
        }
        Command::Private(command) => match private::run(&command) {
            Ok(lines) => print(&lines.join("\n")),
            Err(error) => fail(&error),
        },
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
        Command::Openapi { typescript } => match services::contract_gateway() {
            Ok(gateway) if typescript => {
                print(vellum_api::typescript::client(gateway.openapi()).trim_end())
            }
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
            // The recorder fills the buffer behind Admin → Requests.
            let observability = observability::Observability::new();
            let recorder = Box::new(observability.recorder());
            let _guard = match telemetry::init_with(&telemetry_config, Some(recorder)) {
                Ok(guard) => guard,
                Err(error) => return fail(&error),
            };
            // Resident memory and threads, for the saturation panels.
            vellum_observe::process::register_metrics();
            match serve::run(config.as_deref(), observability) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    tracing::error!(error = %error, "vellum serve failed");
                    fail(&error)
                }
            }
        }
    }
}
