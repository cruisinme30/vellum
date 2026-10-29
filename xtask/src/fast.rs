// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `cargo xtask fast`: the "Fast suite" CI workflow, run locally.
//!
//! Each step runs the same command as `.github/workflows/ci.yml`. A step
//! whose tool is not installed is skipped and named in the summary, so a
//! contributor can still push; CI runs every step regardless.
//!
//! Like CI, the Rust and web steps are skipped when the branch does not touch
//! them; `scripts/ci/changed-areas.sh` decides for both (ADR 0035, D139).

use std::env;
use std::ffi::OsString;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

/// The part of the repository a step checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Area {
    Rust,
    Web,
    Desktop,
    /// Always runs: files, docs, secrets and commits.
    Any,
}

/// Which areas the branch changed, as `scripts/ci/changed-areas.sh` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Areas {
    rust: bool,
    web: bool,
    desktop: bool,
}

impl Areas {
    const ALL: Self = Self {
        rust: true,
        web: true,
        desktop: true,
    };

    /// Parses the script's `rust=true` / `web=false` lines.
    fn parse(output: &str) -> Self {
        let flag = |key: &str| {
            output
                .lines()
                .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
                .is_none_or(|value| value.trim() != "false")
        };
        Self {
            rust: flag("rust"),
            web: flag("web"),
            desktop: flag("desktop"),
        }
    }

    const fn includes(self, area: Area) -> bool {
        match area {
            Area::Rust => self.rust,
            Area::Web => self.web,
            Area::Desktop => self.desktop,
            Area::Any => true,
        }
    }
}

/// One check: a program, its arguments and where to run it.
struct Step {
    name: &'static str,
    area: Area,
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
            area: Area::Any,
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

    const fn area(mut self, area: Area) -> Self {
        self.area = area;
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
    /// The branch does not change what the step checks; CI skips it too.
    NotNeeded,
}

/// Runs every fast check and fails if any check that ran failed.
pub fn run() -> Result<(), String> {
    let root = repo_root()?;
    let base = merge_base(&root);
    let areas = base
        .as_deref()
        .map_or(Areas::ALL, |base| changed_areas(&root, base));
    let mut results = Vec::new();
    let started = Instant::now();

    for step in steps(base.as_deref(), current_branch(&root).as_deref()) {
        let outcome = if areas.includes(step.area) {
            execute(&root, &step)
        } else {
            Outcome::NotNeeded
        };
        results.push((step.name, outcome));
    }
    let (boundaries, help) = if areas.rust {
        (boundaries_outcome(), help_outcome())
    } else {
        (Outcome::NotNeeded, Outcome::NotNeeded)
    };
    results.push(("Crate boundaries", boundaries));
    results.push(("Help books and their corpus", help));

    report(&results, started.elapsed().as_secs());
    if results
        .iter()
        .any(|(_, outcome)| *outcome == Outcome::Failed)
    {
        return Err("Fast suite failed locally; fix the steps above before pushing.".to_owned());
    }
    Ok(())
}

/// The Web job's steps, run in `web/`.
fn web_steps() -> Vec<Step> {
    [
        ("Web install", &["install", "--frozen-lockfile"][..]),
        ("Web format", &["run", "format:check"]),
        ("Web types", &["run", "typecheck"]),
        ("Web lint", &["run", "lint"]),
        ("Web dead code", &["run", "knip"]),
        ("Web tests with coverage", &["run", "test:coverage"]),
        ("Web dependency audit", &["audit"]),
    ]
    .into_iter()
    .map(|(name, args)| Step::new(name, "pnpm", args).in_dir("web").area(Area::Web))
    .collect()
}

/// The Desktop job's steps, run in `desktop/`.
fn desktop_steps() -> Vec<Step> {
    [
        ("Desktop install", &["install", "--frozen-lockfile"][..]),
        ("Desktop format", &["run", "format:check"]),
        ("Desktop types", &["run", "typecheck"]),
        ("Desktop lint", &["run", "lint"]),
        ("Desktop dead code", &["run", "knip"]),
        ("Desktop tests with coverage", &["run", "test:coverage"]),
        ("Desktop build", &["run", "build"]),
        ("Desktop dependency audit", &["audit"]),
    ]
    .into_iter()
    .map(|(name, args)| {
        Step::new(name, "pnpm", args)
            .in_dir("desktop")
            .area(Area::Desktop)
    })
    .collect()
}

/// The fast suite, in the order CI runs it.
fn steps(base: Option<&str>, branch: Option<&str>) -> Vec<Step> {
    let mut steps = vec![
        // Rust
        Step::new("Rust format", "cargo", &["fmt", "--all", "--check"]).area(Area::Rust),
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
        )
        .area(Area::Rust),
        Step::new(
            "Rust docs",
            "cargo",
            &["doc", "--workspace", "--no-deps", "--locked"],
        )
        .with_env("RUSTDOCFLAGS", "-D warnings")
        .area(Area::Rust),
        Step::new(
            "Rust tests",
            "cargo",
            &[
                "nextest",
                "run",
                "--workspace",
                "--locked",
                "--profile",
                "ci",
            ],
        )
        .needs("cargo-nextest")
        .area(Area::Rust),
        Step::new(
            "Licences, bans, sources and advisories",
            "cargo",
            &["deny", "--locked", "check"],
        )
        .needs("cargo-deny")
        .area(Area::Rust),
        Step::new("RustSec audit", "cargo", &["audit", "--deny", "warnings"])
            .needs("cargo-audit")
            .area(Area::Rust),
    ];
    steps.extend(web_steps());
    steps.extend(desktop_steps());
    steps.extend([
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
        Step::new("Decision records", "scripts/ci/check-adrs.sh", &[]).needs("bash"),
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
    ]);

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
            Outcome::NotNeeded => format!("  skip  {name} (no changes here; CI skips it too)"),
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

/// Which areas the commits since `base` touch. Runs everything if the
/// script or git fails.
fn changed_areas(root: &Path, base: &str) -> Areas {
    let Some(paths) = git(root, &["diff", "--name-only", base, "HEAD"]) else {
        return Areas::ALL;
    };
    let output = Command::new(root.join("scripts/ci/changed-areas.sh"))
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(paths.as_bytes())?;
            }
            child.wait_with_output()
        });
    match output {
        Ok(output) if output.status.success() => {
            Areas::parse(&String::from_utf8_lossy(&output.stdout))
        }
        Ok(_) | Err(_) => Areas::ALL,
    }
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
    fn areas_follow_the_script_output() {
        assert_eq!(
            Areas::parse("rust=false\nweb=true\ndesktop=false\n"),
            Areas {
                rust: false,
                web: true,
                desktop: false
            }
        );
        assert_eq!(Areas::parse(""), Areas::ALL);
    }

    #[test]
    fn rust_and_web_steps_are_tagged() {
        let steps = steps(Some("abc"), None);
        let area = |name: &str| steps.iter().find(|step| step.name == name).map(|s| s.area);
        assert_eq!(area("Clippy"), Some(Area::Rust));
        assert_eq!(area("Rust tests"), Some(Area::Rust));
        assert_eq!(area("Web lint"), Some(Area::Web));
        assert_eq!(area("Prose (Vale)"), Some(Area::Any));
        assert_eq!(area("Desktop lint"), Some(Area::Desktop));
        let docs_only = Areas {
            rust: false,
            web: false,
            desktop: false,
        };
        assert!(!docs_only.includes(Area::Rust));
        assert!(docs_only.includes(Area::Any));
    }

    #[test]
    fn missing_tools_are_skipped() {
        let step = Step::new("Nothing", "vellum-no-such-tool", &[]);
        assert_eq!(execute(Path::new("."), &step), Outcome::Skipped);
    }
}
