// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `cargo xtask relock`: resolves lockfile conflicts after merging
//! `develop`.
//!
//! Starts from `develop`'s `Cargo.lock` and `web/pnpm-lock.yaml`, then lets
//! cargo and pnpm add what this branch's manifests need. Every version
//! `develop` already pins stays pinned, so the result is the smallest change
//! on top of `develop`, whichever side of a merge or rebase it came from.

use std::path::Path;
use std::process::{Command, Stdio};

/// The lockfiles, the base ref to take them from, and the command that
/// re-resolves each one against the current manifests.
const LOCKFILES: [(&str, &str, &[&str], &str); 2] = [
    (
        "Cargo.lock",
        "cargo",
        &["metadata", "--format-version", "1"],
        ".",
    ),
    (
        "web/pnpm-lock.yaml",
        "pnpm",
        &["install", "--lockfile-only"],
        "web",
    ),
];

const BASE: &str = "origin/develop";

/// Regenerates both lockfiles from `develop`'s and stages them.
pub fn run() -> Result<(), String> {
    let root = git_output(&["rev-parse", "--show-toplevel"])?;
    let root = Path::new(root.trim());
    run_quiet(root, "git", &["fetch", "--quiet", "origin", "develop"])?;

    for (lockfile, program, args, dir) in LOCKFILES {
        let base = git_output(&["show", &format!("{BASE}:{lockfile}")])?;
        std::fs::write(root.join(lockfile), base)
            .map_err(|error| format!("could not write {lockfile}: {error}"))?;
        run_quiet(&root.join(dir), program, args)?;
        run_quiet(root, "git", &["add", lockfile])?;
    }
    Ok(())
}

fn git_output(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if !output.status.success() {
        return Err(format!("git {} failed", args.join(" ")));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn run_quiet(dir: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .stdout(Stdio::null())
        .status()
        .map_err(|error| format!("could not run {program}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} {} failed", args.join(" ")))
    }
}
