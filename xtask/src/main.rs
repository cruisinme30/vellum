// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Repository automation, run as `cargo xtask <command>`.
//!
//! - `cargo xtask fast` runs the "Fast suite" CI checks locally. The
//!   pre-push hook runs it before every push.
//! - `cargo xtask pr-check --title <title> --body-file <file>` runs CI's
//!   title, branch, description and linked-issue checks before a pull
//!   request is opened.
//! - `cargo xtask install-hooks` turns on the repository's git hooks and
//!   commit template, and makes `git blame` skip reformatting commits.
//! - `cargo xtask relock` resolves `Cargo.lock` and `web/pnpm-lock.yaml`
//!   conflicts after merging `develop`.
//! - `cargo xtask boundaries` enforces the crate layering: every workspace
//!   crate declares a layer in `[package.metadata.vellum]`, and may depend
//!   only on Vellum crates in strictly lower layers. Crates in the same
//!   layer (for example two services) can never depend on each other, so a
//!   service cannot reach into another service's storage.
//! - `cargo xtask schema [--check]` generates the editor's and the server's
//!   views of the block model from `schema/blocks.json`.
//! - `cargo xtask help` checks the help books in `docs/`, and
//!   `cargo xtask help write` regenerates the files derived from them.
//! - `cargo xtask budgets` checks the binary's size, cold start, idle
//!   memory, idle CPU, memory with ten live rooms and search warm, and
//!   shutdown against the brief's targets.
//! - `cargo xtask adr next` prints the next free decision record and D
//!   numbers, and `cargo xtask adr new <slug>` starts a record with them.
//! - `cargo xtask installers` writes a release's Homebrew formula and
//!   winget manifests from its checksums.
//! - `cargo xtask perf-gate` fails the nightly run when a k6 p95 regresses
//!   by 10 % or more against `deploy/perf/baseline.json`.

use std::process::{Command, ExitCode};

mod adr;
mod boundaries;
mod budgets;
mod fast;
mod help;
mod installers;
mod load;
mod perf_gate;
mod pr_check;
mod relock;
mod schema;

const USAGE: &str = "usage: cargo xtask <fast | pr-check | install-hooks | \
                     relock | boundaries | adr | help [check|write] | budgets \
                     | installers | perf-gate | schema [--check]>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, rest) = args
        .split_first()
        .map_or((None, &[][..]), |(c, r)| (Some(c.as_str()), r));
    let mode = rest.first().map(String::as_str);
    let result = match command {
        Some("fast") => fast::run(),
        Some("pr-check") => pr_check::run(rest),
        Some("install-hooks") => install_hooks(),
        Some("relock") => relock::run(),
        Some("boundaries") => boundaries::run(),
        Some("adr") => adr::run(rest),
        Some("help") => help::run(mode),
        Some("budgets") => budgets::run(rest),
        Some("installers") => installers::run(rest),
        Some("perf-gate") => perf_gate::run(rest),
        Some("schema") => schema::run(rest),
        _ => Err(USAGE.to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            #[expect(
                clippy::print_stderr,
                reason = "xtask reports to the terminal"
            )]
            {
                eprintln!("{message}");
            }
            ExitCode::FAILURE
        }
    }
}

/// Points git at `.githooks` (commit-msg, pre-commit, pre-push), at the
/// commit template and at `.git-blame-ignore-revs`.
fn install_hooks() -> Result<(), String> {
    for (key, value) in [
        ("core.hooksPath", ".githooks"),
        ("commit.template", ".gitmessage"),
        // git blame skips the 80-column reformat (ADR 0074).
        ("blame.ignoreRevsFile", ".git-blame-ignore-revs"),
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
