// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `cargo xtask fast`: the "Fast suite" CI workflow, run locally.
//!
//! Each step runs the same command as `.github/workflows/ci.yml`. A step
//! whose tool is not installed fails the run, because a skipped check is
//! how failures used to reach CI unnoticed; `scripts/dev/install-tools.sh`
//! installs them all. `VELLUM_ALLOW_MISSING_TOOLS=1` turns a missing tool
//! back into a reported skip (ADR 0048, D203).
//!
//! Like CI, the Rust and web steps are skipped when the branch does not touch
//! them; `scripts/ci/changed-areas.sh` decides for both (ADR 0035, D139).
//! The Rust tests run only for the crates the branch changed; CI runs the
//! whole workspace. `VELLUM_FAST_ALL_TESTS=1` tests every crate locally too,
//! and adds the UI layout checks: the integrator's full run (issue #532).

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
    /// The UI layout, focus and loading checks: the web app and the website.
    Ui,
    /// Always runs: files, docs, secrets and commits.
    Any,
}

/// Which areas the branch changed, as `scripts/ci/changed-areas.sh` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one flag per line the script prints"
)]
struct Areas {
    rust: bool,
    web: bool,
    desktop: bool,
    ui: bool,
}

impl Areas {
    const ALL: Self = Self {
        rust: true,
        web: true,
        desktop: true,
        ui: true,
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
            ui: flag("ui"),
        }
    }

    const fn includes(self, area: Area) -> bool {
        match area {
            Area::Rust => self.rust,
            Area::Web => self.web,
            Area::Desktop => self.desktop,
            Area::Ui => self.ui,
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
    /// Nothing for this step to check on this branch.
    not_needed: bool,
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
            not_needed: false,
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
    /// The step's tool is not installed, and missing tools are not allowed.
    Missing,
    /// The branch does not change what the step checks; CI skips it too.
    NotNeeded,
}

/// Set to `1` to report a missing tool as a skip instead of a failure.
const ALLOW_MISSING_TOOLS: &str = "VELLUM_ALLOW_MISSING_TOOLS";
/// Set to `1` to run every crate's tests, as CI does.
const ALL_TESTS: &str = "VELLUM_FAST_ALL_TESTS";

/// What a skipped step means: allowed only when the contributor opted in.
const fn missing_tool(allowed: bool) -> Outcome {
    if allowed {
        Outcome::Skipped
    } else {
        Outcome::Missing
    }
}

fn env_flag(key: &str) -> bool {
    env::var(key).is_ok_and(|value| value == "1")
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
    let allow_missing = env_flag(ALLOW_MISSING_TOOLS);
    let tested = if env_flag(ALL_TESTS) {
        None
    } else {
        base.as_deref()
            .and_then(|base| changed_packages(&root, base))
    };

    for step in steps(
        base.as_deref(),
        current_branch(&root).as_deref(),
        tested.as_deref(),
    ) {
        let outcome = if !areas.includes(step.area) || step.not_needed {
            Outcome::NotNeeded
        } else {
            match execute(&root, &step) {
                Outcome::Skipped => missing_tool(allow_missing),
                outcome => outcome,
            }
        };
        results.push((step.name, outcome));
    }
    let (boundaries, help) = if areas.rust {
        (boundaries_outcome(), help_outcome())
    } else {
        (Outcome::NotNeeded, Outcome::NotNeeded)
    };
    if areas.rust && !docker_available() {
        // The missing tool is Docker, which the Postgres and Meilisearch tests need.
        results.push(("Postgres and Meilisearch tests (Docker)", Outcome::Skipped));
    }
    results.push(("Crate boundaries", boundaries));
    results.push(("Help books and their corpus", help));
    // The block model feeds both the Rust and the web code.
    let schema = if areas.rust || areas.web {
        schema_outcome()
    } else {
        Outcome::NotNeeded
    };
    results.push(("Block model", schema));

    report(&results, started.elapsed().as_secs());
    if results
        .iter()
        .any(|(_, outcome)| *outcome == Outcome::Missing)
    {
        return Err(format!(
            "Some checks could not run because their tool is not installed, so CI \
             would be the first to run them. Install the tools with \
             scripts/dev/install-tools.sh, or set {ALLOW_MISSING_TOOLS}=1 to push anyway."
        ));
    }
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

/// Tests that start Postgres or Meilisearch in a container through
/// testcontainers: two whole `vellum-db` binaries, plus every test named
/// after the engine it starts.
const DOCKER_TESTS: &str = "binary_id(vellum-db::postgres) | binary_id(vellum-db::shards) \
     | test(/postgres/) | test(/meilisearch/)";

/// Whether a Docker daemon looks reachable: `DOCKER_HOST` is set or the
/// default socket exists. Cloud sandboxes have neither.
fn docker_available() -> bool {
    env::var_os("DOCKER_HOST").is_some() || Path::new("/var/run/docker.sock").exists()
}
/// The tests of `packages`, or of the whole workspace when `None`. With no
/// packages the step is not needed.
/// Without Docker, the container-backed database and search tests are left
/// out (and reported as skipped); CI always runs them.
fn rust_tests(docker: bool, packages: Option<&[String]>) -> Step {
    let mut step = Step::new("Rust tests", "cargo", &["nextest", "run"])
        .needs("cargo-nextest")
        .area(Area::Rust);
    match packages {
        None => step.args.push("--workspace".to_owned()),
        Some([]) => step.not_needed = true,
        Some(packages) => {
            for package in packages {
                step.args.push("-p".to_owned());
                step.args.push(package.clone());
            }
        }
    }
    for arg in ["--locked", "--profile", "ci"] {
        step.args.push(arg.to_owned());
    }
    if !docker {
        step.args.push("-E".to_owned());
        step.args.push(format!("not ({DOCKER_TESTS})"));
    }
    step
}

/// The fast suite, in the order CI runs it.
fn steps(base: Option<&str>, branch: Option<&str>, tested: Option<&[String]>) -> Vec<Step> {
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
        rust_tests(docker_available(), tested),
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
    // The "UI layout (quick)" job: what a merged batch most often breaks
    // without any single branch failing on its own (issue #532). It takes
    // minutes, so only the integrator's full run (VELLUM_FAST_ALL_TESTS=1)
    // includes it; threads run focused tests.
    let mut ui = Step::new("UI layout (quick)", "scripts/ci/ui-quick.sh", &[])
        .needs("mdbook")
        .area(Area::Ui);
    ui.not_needed = !env_flag(ALL_TESTS);
    steps.push(ui);
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

/// Smaller local builds, so a workspace test build fits a cloud sandbox's
/// disk: no debug info and no incremental cache. A variable the
/// contributor already set wins.
const LOCAL_BUILD_ENV: [(&str, &str); 2] =
    [("CARGO_PROFILE_DEV_DEBUG", "0"), ("CARGO_INCREMENTAL", "0")];

/// The `LOCAL_BUILD_ENV` entries the environment does not already set.
fn local_build_env(is_set: impl Fn(&str) -> bool) -> Vec<(&'static str, &'static str)> {
    LOCAL_BUILD_ENV
        .into_iter()
        .filter(|(key, _)| !is_set(key))
        .collect()
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
    let build_env = if step.program == "cargo" {
        local_build_env(|key| env::var_os(key).is_some())
    } else {
        Vec::new()
    };
    let status = Command::new(program)
        .envs(build_env)
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

fn schema_outcome() -> Outcome {
    progress("==> Block model is generated from schema/");
    match crate::schema::run(&["--check".to_owned()]) {
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
            Outcome::Missing => format!("  MISS  {name} (tool not installed)"),
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
                // `git` output is trimmed; end the last path with a newline.
                stdin.write_all(b"\n")?;
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

/// The workspace crates the commits since `base` touch, by name. `None`
/// when git or `cargo metadata` fails, so the caller tests everything.
fn changed_packages(root: &Path, base: &str) -> Option<Vec<String>> {
    let paths = git(root, &["diff", "--name-only", base, "HEAD"]).unwrap_or_default();
    let metadata = cargo_metadata::MetadataCommand::new()
        .current_dir(root)
        .no_deps()
        .exec()
        .ok()?;
    let crates: Vec<(String, String)> = metadata
        .workspace_packages()
        .into_iter()
        .filter_map(|package| {
            let dir = package.manifest_path.parent()?.strip_prefix(root).ok()?;
            Some((package.name.to_string(), dir.as_str().to_owned()))
        })
        .collect();
    Some(packages_for(paths.lines(), &crates))
}

/// The crates (name, directory relative to the root) that own `paths`. A
/// path belongs to the crate with the deepest directory that contains it;
/// paths outside every crate (the root manifest, docs) select none.
fn packages_for<'a>(
    paths: impl IntoIterator<Item = &'a str>,
    crates: &[(String, String)],
) -> Vec<String> {
    let mut names: Vec<String> = paths
        .into_iter()
        .filter_map(|path| {
            crates
                .iter()
                .filter(|(_, dir)| {
                    !dir.is_empty()
                        && path
                            .strip_prefix(dir.as_str())
                            .is_some_and(|rest| rest.starts_with('/'))
                })
                .max_by_key(|(_, dir)| dir.len())
                .map(|(name, _)| name.clone())
        })
        .collect();
    names.sort();
    names.dedup();
    names
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
        let steps = steps(Some("abc123"), Some("feat/thing"), None);
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
        let names = names(&steps(None, None, None));
        assert!(!names.contains(&"Commit template and DCO sign-off"));
        assert!(!names.contains(&"Secrets"));
        assert!(!names.contains(&"Branch name"));
    }

    #[test]
    fn long_lived_branches_skip_the_name_check() {
        for branch in ["develop", "main"] {
            assert!(!names(&steps(Some("abc"), Some(branch), None)).contains(&"Branch name"));
        }
    }

    #[test]
    fn commit_check_spans_base_to_head() {
        let steps = steps(Some("abc123"), None, None);
        let commits = steps
            .iter()
            .find(|step| step.name == "Commit template and DCO sign-off")
            .expect("commit step");
        assert_eq!(commits.args, ["abc123", "HEAD"]);
    }

    #[test]
    fn areas_follow_the_script_output() {
        assert_eq!(
            Areas::parse("rust=false\nweb=true\ndesktop=false\nui=false\n"),
            Areas {
                rust: false,
                web: true,
                desktop: false,
                ui: false,
            }
        );
        assert_eq!(Areas::parse(""), Areas::ALL);
    }

    #[test]
    fn rust_and_web_steps_are_tagged() {
        let steps = steps(Some("abc"), None, None);
        let area = |name: &str| steps.iter().find(|step| step.name == name).map(|s| s.area);
        assert_eq!(area("Clippy"), Some(Area::Rust));
        assert_eq!(area("Rust tests"), Some(Area::Rust));
        assert_eq!(area("Web lint"), Some(Area::Web));
        assert_eq!(area("Prose (Vale)"), Some(Area::Any));
        assert_eq!(area("Desktop lint"), Some(Area::Desktop));
        assert_eq!(area("UI layout (quick)"), Some(Area::Ui));
        let docs_only = Areas {
            rust: false,
            web: false,
            desktop: false,
            ui: false,
        };
        assert!(!docs_only.includes(Area::Rust));
        assert!(docs_only.includes(Area::Any));
    }

    #[test]
    fn missing_tools_are_skipped() {
        let step = Step::new("Nothing", "vellum-no-such-tool", &[]);
        assert_eq!(execute(Path::new("."), &step), Outcome::Skipped);
    }

    #[test]
    fn missing_tools_fail_unless_allowed() {
        assert_eq!(missing_tool(false), Outcome::Missing);
        assert_eq!(missing_tool(true), Outcome::Skipped);
    }

    #[test]
    fn rust_tests_cover_only_the_changed_crates() {
        let all = rust_tests(true, None);
        assert!(all.args.contains(&"--workspace".to_owned()));
        let some = rust_tests(true, Some(&["vellum-db".to_owned()]));
        assert_eq!(some.args[..4], ["nextest", "run", "-p", "vellum-db"]);
        assert!(!some.args.contains(&"--workspace".to_owned()));
        assert!(rust_tests(true, Some(&[])).not_needed);
    }

    #[test]
    fn changed_paths_map_to_their_crates() {
        let crates = [
            ("vellum-db".to_owned(), "crates/vellum-db".to_owned()),
            ("vellum-dbx".to_owned(), "crates/vellum-dbx".to_owned()),
            ("xtask".to_owned(), "xtask".to_owned()),
            ("inner".to_owned(), "crates/vellum-db/inner".to_owned()),
        ];
        let paths = [
            "crates/vellum-db/src/lib.rs",
            "crates/vellum-db/tests/jobs.rs",
            "crates/vellum-db/inner/src/lib.rs",
            "xtask/src/fast.rs",
            "Cargo.lock",
            "docs/adr/0048-x.md",
        ];
        assert_eq!(
            packages_for(paths, &crates),
            ["inner", "vellum-db", "xtask"]
        );
    }

    #[test]
    fn rust_tests_leave_out_container_tests_only_without_docker() {
        let with = rust_tests(true, None);
        assert!(!with.args.iter().any(|arg| arg == "-E"));
        let without = rust_tests(false, None);
        assert_eq!(
            without.args[without.args.len() - 2..],
            ["-E".to_owned(), format!("not ({DOCKER_TESTS})")]
        );
    }

    #[test]
    fn the_last_changed_path_counts() {
        // `git diff` output is trimmed, so the last path has no newline.
        let root = repo_root().expect("inside the repository");
        let output = Command::new(root.join("scripts/ci/changed-areas.sh"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(b"README.md\nxtask/src/fast.rs")?;
                }
                child.wait_with_output()
            })
            .expect("script runs");
        let areas = Areas::parse(&String::from_utf8_lossy(&output.stdout));
        assert!(areas.rust, "{areas:?}");
    }

    #[test]
    fn local_builds_drop_debug_info_unless_already_set() {
        assert_eq!(local_build_env(|_| false), LOCAL_BUILD_ENV.to_vec());
        assert_eq!(
            local_build_env(|key| key == "CARGO_INCREMENTAL"),
            [("CARGO_PROFILE_DEV_DEBUG", "0")]
        );
    }
}
