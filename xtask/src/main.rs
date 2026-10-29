// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Repository automation, run as `cargo xtask <command>`.
//!
//! - `cargo xtask fast` runs the "Fast suite" CI checks locally. The
//!   pre-push hook runs it before every push.
//! - `cargo xtask install-hooks` turns on the repository's git hooks and
//!   commit template.
//! - `cargo xtask boundaries` enforces the crate layering: every workspace
//!   crate declares a layer in `[package.metadata.vellum]`, and may depend
//!   only on Vellum crates in strictly lower layers. Crates in the same
//!   layer (for example two services) can never depend on each other, so a
//!   service cannot reach into another service's storage.

use std::process::{Command, ExitCode};

mod boundaries;
mod fast;

const USAGE: &str = "usage: cargo xtask <fast | install-hooks | boundaries>";

fn main() -> ExitCode {
    let command = std::env::args().nth(1);
    let result = match command.as_deref() {
        Some("fast") => fast::run(),
        Some("install-hooks") => install_hooks(),
        Some("boundaries") => boundaries::run(),
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
