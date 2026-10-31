// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `cargo xtask pr-check`: the pull request checks CI runs on the title,
//! branch and description, run before the pull request is opened
//! (ADR 0048, D204).
//!
//! ```sh
//! cargo xtask pr-check --title "feat(db): ..." --body-file pr.md [--base develop]
//! ```
//!
//! It runs the same scripts as the "Commits and description" and "Issue
//! and decision record" jobs in `.github/workflows/ci.yml`, so a title over
//! 72 characters, a missing description section or a "Closes #N" that
//! points at a pull request fails here instead of in CI.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

const USAGE: &str = "usage: cargo xtask pr-check --title <title> --body-file \
                     <file> [--base <branch>] [--branch <branch>]";

/// The repository whose issues `Closes #N` must name.
const DEFAULT_REPO: &str = "cruisinme30/vellum";

/// What the pull request will say.
#[derive(Debug, PartialEq, Eq)]
struct Request {
    title: String,
    body_file: String,
    base: String,
    branch: Option<String>,
}

fn parse(args: &[String]) -> Result<Request, String> {
    let mut title = None;
    let mut body_file = None;
    let mut base = "develop".to_owned();
    let mut branch = None;
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("{flag} needs a value\n{USAGE}"))?
            .clone();
        match flag.as_str() {
            "--title" => title = Some(value),
            "--body-file" => body_file = Some(value),
            "--base" => base = value,
            "--branch" => branch = Some(value),
            _ => return Err(format!("unknown option {flag}\n{USAGE}")),
        }
    }
    Ok(Request {
        title: title.ok_or_else(|| USAGE.to_owned())?,
        body_file: body_file.ok_or_else(|| USAGE.to_owned())?,
        base,
        branch,
    })
}

pub fn run(args: &[String]) -> Result<(), String> {
    let request = parse(args)?;
    let root = git(Path::new("."), &["rev-parse", "--show-toplevel"])
        .ok_or("cargo xtask pr-check must run inside the Vellum repository")?;
    let root = Path::new(&root);
    let body = fs::read_to_string(&request.body_file).map_err(|error| {
        format!("cannot read {}: {error}", request.body_file)
    })?;
    let branch = request
        .branch
        .clone()
        .or_else(|| git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"]))
        .ok_or("cannot tell the branch; pass --branch")?;
    let base_sha = git(
        root,
        &["merge-base", &format!("origin/{}", request.base), "HEAD"],
    )
    .ok_or_else(|| {
        format!("cannot find origin/{}; run git fetch first", request.base)
    })?;
    let repo = env::var("GH_REPO").unwrap_or_else(|_| DEFAULT_REPO.to_owned());

    let mut failed = Vec::new();

    let names = names_script(&request.base, &branch, &request.title);
    if !bash(root, &names, &[]) {
        failed.push("Branch name and title");
    }
    if !bash(
        root,
        "scripts/ci/check-pr-body.sh",
        &[("PR_BODY", body.as_str())],
    ) {
        failed.push("Description follows the template");
    }
    let token = issue_lookup_token(root, &repo);
    let mut trace_env = vec![
        ("PR_BODY", body.as_str()),
        ("TITLE", request.title.as_str()),
        ("BASE", request.base.as_str()),
        ("BASE_SHA", base_sha.as_str()),
        ("HEAD_SHA", "HEAD"),
        ("GH_REPO", repo.as_str()),
    ];
    match &token {
        Lookup::AsIs => {}
        Lookup::Anonymous => trace_env.push(("GH_TOKEN", "")),
        Lookup::Unavailable => trace_env.push(("ISSUE_LOOKUP", "off")),
    }
    if !bash(root, "scripts/ci/check-traceability.sh", &trace_env) {
        failed.push("Linked issue, and a decision record for features");
    }

    if failed.is_empty() {
        report(
            "The pull request passes CI's title, branch, description and issue \
                checks.",
        );
        Ok(())
    } else {
        Err(format!(
            "Fix these before opening the pull request: {}.",
            failed.join(", ")
        ))
    }
}

/// The CI step "Branch name and title follow the conventions", with the
/// values passed as arguments so no quoting can break it.
fn names_script(base: &str, branch: &str, title: &str) -> String {
    let quote = |value: &str| format!("'{}'", value.replace('\'', r"'\''"));
    format!(
        "source scripts/ci/commit-rules.sh\nbase={}; branch={}; title={}\n\
         status=0\n\
         if [[ \"$base\" == main && \"$branch\" == develop ]]; then :\n\
         elif ! check_branch \"$branch\"; then\n\
         echo \"Branch '$branch' must be named <type>/<topic>, e.g. \
         feat/typed-ids\" >&2; status=1\nfi\n\
         if ! check_subject \"$title\"; then\n\
         echo \"Title is not a Conventional Commit of at most 72 characters \
         after the type: $title\" >&2; status=1\nfi\nexit $status",
        quote(base),
        quote(branch),
        quote(title)
    )
}

/// How `gh` can look the linked issues up.
enum Lookup {
    /// The configured credentials work.
    AsIs,
    /// They do not, but the repository is public, so no token works.
    Anonymous,
    /// GitHub cannot be reached; the issue is checked in CI only.
    Unavailable,
}

fn issue_lookup_token(root: &Path, repo: &str) -> Lookup {
    let probe = |anonymous: bool| {
        let mut command = Command::new("gh");
        let command = command
            .args(["api", &format!("repos/{repo}"), "--silent"])
            .current_dir(root);
        let command = if anonymous {
            command.env("GH_TOKEN", "")
        } else {
            command
        };
        command.output().is_ok_and(|output| output.status.success())
    };
    if probe(false) {
        Lookup::AsIs
    } else if probe(true) {
        Lookup::Anonymous
    } else {
        report(
            "GitHub is not reachable with gh; the linked issue is checked in \
                CI only.",
        );
        Lookup::Unavailable
    }
}

fn bash(root: &Path, script: &str, envs: &[(&str, &str)]) -> bool {
    let mut command = Command::new("bash");
    let command = if script.starts_with("scripts/") {
        command.arg(root.join(script))
    } else {
        command.args(["-c", script])
    };
    command
        .current_dir(root)
        .envs(envs.iter().copied())
        .status()
        .is_ok_and(|status| status.success())
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn report(message: &str) {
    #[expect(clippy::print_stderr, reason = "xtask reports to the terminal")]
    {
        eprintln!("{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|&arg| arg.to_owned()).collect()
    }

    fn root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace root")
            .to_path_buf()
    }

    #[test]
    fn parses_flags_with_develop_as_the_default_base() {
        let request =
            parse(&args(&["--title", "ci: x", "--body-file", "pr.md"]))
                .unwrap();
        assert_eq!(request.base, "develop");
        assert_eq!(request.title, "ci: x");
        assert!(parse(&args(&["--title", "ci: x"])).is_err());
        assert!(parse(&args(&["--title"])).is_err());
        assert!(parse(&args(&["--colour", "red"])).is_err());
    }

    #[test]
    fn names_follow_the_ci_rules() {
        let check = |base: &str, branch: &str, title: &str| {
            bash(&root(), &names_script(base, branch, title), &[])
        };
        assert!(check("develop", "ci/catch-failures", "ci: catch failures"));
        assert!(check("main", "develop", "chore(release): cut"));
        assert!(!check("develop", "claude/project-thread-x", "ci: x"));
        let long = format!("feat(web): {}", "a".repeat(80));
        assert!(!check("develop", "feat/x", &long));
        // Quotes in a title cannot escape the script.
        assert!(!check("develop", "feat/x", "it's'; exit 0; '"));
    }
}
