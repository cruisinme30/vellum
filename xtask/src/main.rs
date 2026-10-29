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
//! - `cargo xtask schema [--check]` generates the editor's and the server's
//!   views of the block model from `schema/blocks.json`.

use std::process::{Command, ExitCode};

mod boundaries;
mod fast;
mod relock;
mod schema;

const USAGE: &str =
    "usage: cargo xtask <fast | install-hooks | relock | boundaries | schema [--check]>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.split_first() {
        Some((command, [])) if command == "fast" => fast::run(),
        Some((command, [])) if command == "install-hooks" => install_hooks(),
        Some((command, [])) if command == "relock" => relock::run(),
        Some((command, [])) if command == "boundaries" => boundaries::run(),
        Some((command, rest)) if command == "schema" => schema::run(rest),
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
