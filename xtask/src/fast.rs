// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `cargo xtask fast`: the "Fast suite" CI workflow, run locally.
//!
//! Each step runs the same command as `.github/workflows/ci.yml`. A step
//! whose tool is not installed is skipped and named in the summary, so a
//! contributor can still push; CI runs every step regardless.

use std::env;
use std::ffi::OsString;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// One check: a program, its arguments and where to run it.
struct Step {
    name: &'static str,
    /// The executable that must be on `PATH` for the step to run.
    tool: &'static str,
    program: &'static str,
    args: Vec<String>,
    /// Directory relative to the repository root.
    dir: &'static str,
    env: Vec<(&'static str, &'static str)>,
}

impl Step {
    fn new(name: &'static str, program: &'static str, args: &[&str]) -> Self {
        Self {
            name,
            tool: program,
            program,
            args: args.iter().map(|&arg| arg.to_owned()).collect(),
            dir: ".",
            env: Vec::new(),
        }
    }

    /// Requires a different executable than the program, such as
    /// `cargo-nextest` for `cargo nextest`.
    const fn needs(mut self, tool: &'static str) -> Self {
        self.tool = tool;
        self
    }

    const fn in_dir(mut self, dir: &'static str) -> Self {
        self.dir = dir;
        self
    }

    fn with_env(mut self, key: &'static str, value: &'static str) -> Self {
        self.env.push((key, value));
        self
    }
}

/// How a step ended.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Passed,
    Failed,
    Skipped,
}

/// Runs every fast check and fails if any check that ran failed.
pub fn run() -> Result<(), String> {
    let root = repo_root()?;
    let base = merge_base(&root);
    let mut results = Vec::new();
    let started = Instant::now();

    for step in steps(base.as_deref(), current_branch(&root).as_deref()) {
        let outcome = execute(&root, &step);
        results.push((step.name, outcome));
    }
    results.push(("Crate boundaries", boundaries_outcome()));
    results.push(("Help books and their corpus", help_outcome()));

    report(&results, started.elapsed().as_secs());
    if results
        .iter()
        .any(|(_, outcome)| *outcome == Outcome::Failed)
    {
        return Err("Fast suite failed locally; fix the steps above before pushing.".to_owned());
    }
    Ok(())
}

/// The fast suite, in the order CI runs it.
fn steps(base: Option<&str>, branch: Option<&str>) -> Vec<Step> {
    let mut steps = vec![
        // Rust
        Step::new("Rust format", "cargo", &["fmt", "--all", "--check"]),
        // `-D warnings` after `--` also denies rustc warnings in every
        // target, matching CI's RUSTFLAGS without invalidating the
        // contributor's normal build cache.
        Step::new(
            "Clippy",
            "cargo",
            &[
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
        ),
        Step::new(
            "Rust docs",
            "cargo",
            &["doc", "--workspace", "--no-deps", "--locked"],
        )
        .with_env("RUSTDOCFLAGS", "-D warnings"),
        Step::new(
            "Rust tests",
            "cargo",
            &["nextest", "run", "--workspace", "--locked"],
        )
        .needs("cargo-nextest"),
        Step::new(
            "Licences, bans, sources and advisories",
            "cargo",
            &["deny", "--locked", "check"],
        )
        .needs("cargo-deny"),
        Step::new("RustSec audit", "cargo", &["audit", "--deny", "warnings"]).needs("cargo-audit"),
        // Web
        Step::new("Web install", "pnpm", &["install", "--frozen-lockfile"]).in_dir("web"),
        Step::new("Web format", "pnpm", &["run", "format:check"]).in_dir("web"),
        Step::new("Web types", "pnpm", &["run", "typecheck"]).in_dir("web"),
        Step::new("Web lint", "pnpm", &["run", "lint"]).in_dir("web"),
        Step::new("Web dead code", "pnpm", &["run", "knip"]).in_dir("web"),
        Step::new("Web tests with coverage", "pnpm", &["run", "test:coverage"]).in_dir("web"),
        Step::new("Web dependency audit", "pnpm", &["audit"]).in_dir("web"),
        // Files, docs and secrets
        Step::new("REUSE headers", "reuse", &["lint"]),
        Step::new("YAML", "yamllint", &["--strict", "."]),
        Step::new(
            "Shell",
            "bash",
            &[
                "-c",
                "shellcheck -x scripts/*/*.sh deploy/*/*.sh .githooks/*",
            ],
        )
        .needs("shellcheck"),
        Step::new("Markdown", "markdownlint-cli2", &[]),
        Step::new(
            "Prose (Vale)",
            "vale",
            &[
                "--glob=!{**/node_modules/**,target/**,LICENSES/**,LICENSE,corpus/**}",
                ".",
            ],
        ),
        Step::new(
            "Local links",
            "lychee",
            &[
                "--offline",
                "--no-progress",
                "--include-fragments",
                "--exclude-path",
                "LICENSES",
                "--exclude-path",
                "web/node_modules",
                "--exclude-path",
                "corpus",
                "**/*.md",
            ],
        ),
        Step::new(
            "SQL, Docker, Compose, Helm and Kubernetes",
            "scripts/ci/lint-infra.sh",
            &[],
        )
        .needs("bash"),
    ];

    if let Some(base) = base {
        let range = format!("--log-opts={base}..HEAD");
        let mut secrets = Step::new("Secrets", "gitleaks", &["git", "--no-banner", "--redact"]);
        secrets.args.push(range);
        steps.push(secrets);

        let mut commits = Step::new(
            "Commit template and DCO sign-off",
            "scripts/ci/check-commits.sh",
            &[],
        )
        .needs("bash");
        commits.args = vec![base.to_owned(), "HEAD".to_owned()];
        steps.push(commits);
    }

    if let Some(branch) = branch.filter(|name| !matches!(*name, "develop" | "main")) {
        let mut name = Step::new(
            "Branch name",
            "bash",
            &[
                "-c",
                "source scripts/ci/commit-rules.sh && check_branch \"$1\" \
                 || { echo \"Branch '$1' must be named <type>/<topic>\" >&2; exit 1; }",
                "branch-name",
            ],
        );
        name.args.push(branch.to_owned());
        steps.push(name);
    }

    steps
}

fn execute(root: &Path, step: &Step) -> Outcome {
    if !on_path(step.tool) && !root.join(step.program).is_file() {
        return Outcome::Skipped;
    }
    progress(&format!("==> {}", step.name));
    let program = if step.program.starts_with("scripts/") {
        root.join(step.program).into_os_string()
    } else {
        OsString::from(step.program)
    };
    let status = Command::new(program)
        .args(&step.args)
        .current_dir(root.join(step.dir))
        .envs(step.env.iter().copied())
        .status();
    match status {
        Ok(status) if status.success() => Outcome::Passed,
        Ok(_) | Err(_) => Outcome::Failed,
    }
}

fn boundaries_outcome() -> Outcome {
    progress("==> Crate boundaries");
    match crate::boundaries::run() {
        Ok(()) => Outcome::Passed,
        Err(message) => {
            progress(&message);
            Outcome::Failed
        }
    }
}

fn help_outcome() -> Outcome {
    progress("==> Help books and their corpus");
    match crate::help::run(None) {
        Ok(()) => Outcome::Passed,
        Err(message) => {
            progress(&message);
            Outcome::Failed
        }
    }
}

fn report(results: &[(&str, Outcome)], seconds: u64) {
    let mut lines = vec![
        String::new(),
        format!("Fast suite, run locally in {seconds} s:"),
    ];
    for (name, outcome) in results {
        lines.push(match outcome {
            Outcome::Passed => format!("  pass  {name}"),
            Outcome::Failed => format!("  FAIL  {name}"),
            Outcome::Skipped => format!("  skip  {name} (tool not installed; CI still runs it)"),
        });
    }
    progress(&lines.join("\n"));
}

/// Writes to standard error, where the child processes report too.
fn progress(message: &str) {
    let mut stderr = io::stderr().lock();
    let _ignored = writeln!(stderr, "{message}");
}

fn repo_root() -> Result<PathBuf, String> {
    git(Path::new("."), &["rev-parse", "--show-toplevel"])
        .map(PathBuf::from)
        .ok_or_else(|| "cargo xtask fast must run inside the Vellum repository".to_owned())
}

/// The commit where the current branch left `develop`, if known.
fn merge_base(root: &Path) -> Option<String> {
    git(root, &["merge-base", "origin/develop", "HEAD"])
}

fn current_branch(root: &Path) -> Option<String> {
    git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(text.trim().to_owned()).filter(|value| !value.is_empty())
}

fn on_path(tool: &str) -> bool {
    env::var_os("PATH")
        .is_some_and(|paths| env::split_paths(&paths).any(|dir| dir.join(tool).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(steps: &[Step]) -> Vec<&'static str> {
        steps.iter().map(|step| step.name).collect()
    }

    #[test]
    fn mirrors_every_fast_suite_job() {
        let steps = steps(Some("abc123"), Some("feat/thing"));
        let names = names(&steps);
        for expected in [
            "Rust format",
            "Clippy",
            "Rust tests",
            "Web lint",
            "REUSE headers",
            "Secrets",
            "Commit template and DCO sign-off",
            "Branch name",
        ] {
            assert!(names.contains(&expected), "missing step {expected}");
        }
    }

    #[test]
    fn commit_checks_need_a_base() {
        let names = names(&steps(None, None));
        assert!(!names.contains(&"Commit template and DCO sign-off"));
        assert!(!names.contains(&"Secrets"));
        assert!(!names.contains(&"Branch name"));
    }

    #[test]
    fn long_lived_branches_skip_the_name_check() {
        for branch in ["develop", "main"] {
            assert!(!names(&steps(Some("abc"), Some(branch))).contains(&"Branch name"));
        }
    }

    #[test]
    fn commit_check_spans_base_to_head() {
        let steps = steps(Some("abc123"), None);
        let commits = steps
            .iter()
            .find(|step| step.name == "Commit template and DCO sign-off")
            .expect("commit step");
        assert_eq!(commits.args, ["abc123", "HEAD"]);
    }

    #[test]
    fn missing_tools_are_skipped() {
        let step = Step::new("Nothing", "vellum-no-such-tool", &[]);
        assert_eq!(execute(Path::new("."), &step), Outcome::Skipped);
    }
}
