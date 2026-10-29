// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Repository automation, run as `cargo xtask <command>`.
//!
//! - `cargo xtask fast` runs the "Fast suite" CI checks locally. The
//!   pre-push hook runs it before every push.
//! - `cargo xtask install-hooks` turns on the repository's git hooks and
//!   commit template.
//! - `cargo xtask relock` resolves `Cargo.lock` and `web/pnpm-lock.yaml`
//!   conflicts after merging `develop`.
//! - `cargo xtask boundaries` enforces the crate layering: every workspace
//!   crate declares a layer in `[package.metadata.vellum]`, and may depend
//!   only on Vellum crates in strictly lower layers. Crates in the same
//!   layer (for example two services) can never depend on each other, so a
//!   service cannot reach into another service's storage.
//! - `cargo xtask help` checks the help books in `docs/`, and
//!   `cargo xtask help write` regenerates the files derived from them.

use std::process::{Command, ExitCode};

mod boundaries;
mod fast;
mod help;
mod relock;

const USAGE: &str =
    "usage: cargo xtask <fast | install-hooks | relock | boundaries | help [check|write]>";

fn main() -> ExitCode {
    let command = std::env::args().nth(1);
    let mode = std::env::args().nth(2);
    let result = match command.as_deref() {
        Some("fast") => fast::run(),
        Some("install-hooks") => install_hooks(),
        Some("relock") => relock::run(),
        Some("boundaries") => boundaries::run(),
        Some("help") => help::run(mode.as_deref()),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            #[expect(clippy::print_stderr, reason = "xtask reports to the terminal")]
            {
                eprintln!("{message}");
            }
            ExitCode::FAILURE
        }
    }
}

/// Points git at `.githooks` (commit-msg, pre-commit, pre-push) and at the
/// commit template.
fn install_hooks() -> Result<(), String> {
    for (key, value) in [
        ("core.hooksPath", ".githooks"),
        ("commit.template", ".gitmessage"),
    ] {
        let status = Command::new("git")
            .args(["config", key, value])
            .status()
            .map_err(|error| format!("could not run git: {error}"))?;
        if !status.success() {
            return Err(format!("git config {key} {value} failed"));
        }
    }
    Ok(())
}
