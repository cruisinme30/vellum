// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The `vellum` binary.
//!
//! One binary runs every Vellum service. Subcommands (`serve`, `reindex`,
//! `mcp`, `docs` and the rest) arrive with their workstreams.

mod reindex;

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

#[derive(Debug, Subcommand)]
enum Command {
    /// Rebuild the index from the files in the data folder.
    Reindex(reindex::Args),
}

fn main() -> ExitCode {
    let Cli { command } = Cli::parse();
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
    match command {
        Some(Command::Reindex(args)) => reindex::run(&args),
        None => {
            tracing::info!("vellum started; no command given");
            ExitCode::SUCCESS
        }
    }
}
