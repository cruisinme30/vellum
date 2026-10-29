// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The `vellum` binary.
//!
//! One binary runs every Vellum service. `vellum serve` starts the gateway
//! with every service registered in [`services`]; `vellum openapi` and
//! `vellum graphql-schema` print the API contracts the clients are
//! generated from. `vellum docs` reads the help books built into the
//! binary; `vellum mcp` serves the MCP server over stdio. Further
//! subcommands (`reindex` and the rest) arrive with their workstreams.

mod docs;
mod mcp;
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
    /// Prints the OpenAPI 3.1 document of every service.
    Openapi,
    /// Prints the `GraphQL` schema of every service.
    GraphqlSchema,
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
