// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The `vellum` binary.
//!
//! One binary runs every Vellum service. Subcommands (`serve`, `reindex`,
//! `mcp` and the rest) arrive with their workstreams. `vellum docs` reads
//! the help books built into the binary.

use std::process::ExitCode;

use clap::{Parser, Subcommand};
use vellum_kernel::telemetry::{self, TelemetryConfig};

mod docs;

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Some(Command::Docs(args)) = &cli.command {
        // Reading the embedded books needs no telemetry or configuration.
        return match docs::run(args, &mut std::io::stdout().lock()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                #[expect(clippy::print_stderr, reason = "the terminal is the interface")]
                {
                    eprintln!("vellum docs: {error}");
                }
                ExitCode::FAILURE
            }
        };
    }
    let config = TelemetryConfig::from_env("vellum", env!("CARGO_PKG_VERSION"));
    let _guard = match telemetry::init(&config) {
        Ok(guard) => guard,
        Err(error) => {
            // Telemetry is not running, so standard error is the only
            // channel left.
            #[expect(clippy::print_stderr, reason = "no subscriber exists yet")]
            {
                eprintln!("vellum: {error}");
            }
            return ExitCode::FAILURE;
        }
    };
    let span = tracing::info_span!("vellum.cli", version = env!("CARGO_PKG_VERSION"));
    let _entered = span.enter();
    tracing::info!("vellum started; no command given");
    ExitCode::SUCCESS
}
