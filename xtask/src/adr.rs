// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `cargo xtask adr`: hands out decision record and decision numbers
//! without asking anyone (ADR 0049, D216).
//!
//! - `cargo xtask adr next` prints the next free ADR and D numbers, looking
//!   at `docs/adr/` in the working tree and on every branch on `origin`.
//! - `cargo xtask adr new <slug> [--decisions N] [--issue N]` writes
//!   `docs/adr/NNNN-<slug>.md` from the template with those numbers filled
//!   in. Commit and push it straight away: a pushed branch is the claim.
//!
//! Two threads that pick numbers before either pushes can still collide;
//! `scripts/ci/check-adrs.sh` fails on a duplicate ADR or D number, so the
//! integrator's batch catches it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

const USAGE: &str =
    "usage: cargo xtask adr <next | new <slug> [--decisions N] [--issue N]>";
const TEMPLATE: &str = "docs/adr/0000-template.md";
const RESERVED: &str = "docs/adr/reserved.toml";

/// Numbers already taken.
#[derive(Debug, Default, PartialEq, Eq)]
struct Taken {
    adrs: BTreeSet<u32>,
    decisions: BTreeSet<u32>,
}

impl Taken {
    fn next_adr(&self) -> u32 {
        self.adrs.last().map_or(1, |last| last + 1)
    }

    fn next_decision(&self) -> u32 {
        self.decisions.last().map_or(1, |last| last + 1)
    }

    /// Adds the ranges in `docs/adr/reserved.toml`.
    fn reserve(&mut self, text: &str) {
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_matches('"');
            match key.trim() {
                "decisions" => {
                    self.decisions.extend(range(value.trim_start_matches('D')));
                }
                "adrs" => self.adrs.extend(range(value)),
                _ => {}
            }
        }
    }

    /// Adds one record, given its file name and contents.
    fn add(&mut self, file_name: &str, text: &str) {
        if let Some(number) = adr_number(file_name).filter(|n| *n > 0) {
            let _ = self.adrs.insert(number);
            self.decisions.extend(decision_numbers(text));
        }
    }
}

pub fn run(args: &[String]) -> Result<(), String> {
    let (command, rest) = args.split_first().ok_or_else(|| USAGE.to_owned())?;
    match command.as_str() {
        "next" => {
            let taken = taken_everywhere()?;
            say(&format!(
                "Next free: ADR {:04}, D{}.\n\
                 Push the branch with the new record right away; that is what \
                 claims the numbers.",
                taken.next_adr(),
                taken.next_decision()
            ));
            Ok(())
        }
        "new" => new_record(rest),
        _ => Err(USAGE.to_owned()),
    }
}

fn new_record(args: &[String]) -> Result<(), String> {
    let mut slug = None;
    let mut count = 3_u32;
    let mut issue = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--decisions" => {
                count = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .filter(|value| (1..=6).contains(value))
                    .ok_or("--decisions takes a number from 1 to 6")?;
            }
            "--issue" => {
                let value =
                    args.next().ok_or("--issue takes an issue number")?;
                issue = Some(value.trim_start_matches('#').to_owned());
            }
            other if slug.is_none() && is_slug(other) => {
                slug = Some(other.to_owned());
            }
            _ => return Err(USAGE.to_owned()),
        }
    }
    let slug = slug.ok_or_else(|| USAGE.to_owned())?;

    let root = git(&["rev-parse", "--show-toplevel"])?;
    let root = Path::new(root.trim());
    let taken = taken_everywhere()?;
    let adr = taken.next_adr();
    let first = taken.next_decision();
    let decisions: Vec<String> =
        (first..first + count).map(|d| format!("D{d}")).collect();

    let template = std::fs::read_to_string(root.join(TEMPLATE))
        .map_err(|error| format!("could not read {TEMPLATE}: {error}"))?;
    let text =
        fill_template(&template, adr, &decisions, issue.as_deref(), &today()?);
    let path = root.join(format!("docs/adr/{adr:04}-{slug}.md"));
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    std::fs::write(&path, text)
        .map_err(|error| format!("could not write: {error}"))?;
    say(&format!(
        "Wrote docs/adr/{adr:04}-{slug}.md with {}.\nCommit and push it now to \
         claim the numbers, then record them in the project memory registry.",
        decisions.join(", ")
    ));
    Ok(())
}

/// The template with the numbers, date and issue filled in.
fn fill_template(
    template: &str,
    adr: u32,
    decisions: &[String],
    issue: Option<&str>,
    date: &str,
) -> String {
    let mut out = Vec::new();
    let mut headings = decisions.iter();
    let mut seen_heading = false;
    for line in template.lines() {
        let line = if line.starts_with("id: ") {
            format!("id: {}", record_id())
        } else if line.starts_with("title: ") {
            format!("title: \"ADR {adr:04}: Short decision title\"")
        } else if line.starts_with("- Date: ") {
            format!("- Date: {date}")
        } else if line.starts_with("- Status: ") {
            "- Status: proposed".to_owned()
        } else if line.starts_with("- Decisions: ") {
            format!("- Decisions: {}", decisions.join(", "))
        } else if line.starts_with("- Issue: ") {
            format!("- Issue: #{}", issue.unwrap_or("N"))
        } else if line.starts_with("- Pull requests: ") {
            "- Pull requests: to be linked when merged".to_owned()
        } else if let Some(rest) = line.strip_prefix("### `DNN`") {
            // One heading per decision; the template has one.
            seen_heading = true;
            let first = headings.next().map_or("DNN", String::as_str);
            let mut block = format!("### {first}{rest}");
            for d in headings.by_ref() {
                let _ = write!(
                    block,
                    "\n\n- What is decided.\n- Why, in one line.\n\n\
                        ### {d}{rest}"
                );
            }
            block
        } else {
            line.to_owned()
        };
        out.push(line);
    }
    debug_assert!(seen_heading, "the template has a decision heading");
    out.join("\n") + "\n"
}

/// Numbers taken in the working tree and on every branch on `origin`.
fn taken_everywhere() -> Result<Taken, String> {
    let root = git(&["rev-parse", "--show-toplevel"])?;
    let root = Path::new(root.trim());
    // Best effort: offline, the local copies of the branches still count.
    let _ = Command::new("git")
        .current_dir(root)
        .args([
            "fetch",
            "--quiet",
            "--prune",
            "origin",
            "+refs/heads/*:refs/remotes/origin/*",
        ])
        .status();

    let mut taken = Taken::default();
    let dir = root.join("docs/adr");
    let entries = std::fs::read_dir(&dir)
        .map_err(|error| format!("could not read docs/adr: {error}"))?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Ok(text) = std::fs::read_to_string(entry.path()) {
            taken.add(&name, &text);
        }
    }

    if let Ok(text) = std::fs::read_to_string(root.join(RESERVED)) {
        taken.reserve(&text);
    }

    // Every record on every branch, read once per distinct blob.
    let refs =
        git(&["for-each-ref", "--format=%(refname)", "refs/remotes/origin"])?;
    let mut blobs = BTreeMap::new();
    for reference in refs.lines().filter(|r| !r.ends_with("/HEAD")) {
        let Ok(listing) = git(&["ls-tree", reference, "docs/adr/"]) else {
            continue;
        };
        for line in listing.lines() {
            // "<mode> blob <sha>\tdocs/adr/<name>"
            let Some((meta, path)) = line.split_once('\t') else {
                continue;
            };
            let Some(sha) = meta.split_whitespace().nth(2) else {
                continue;
            };
            let name = path.rsplit('/').next().unwrap_or(path);
            if adr_number(name).is_some() {
                let _ = blobs.insert(sha.to_owned(), name.to_owned());
            }
        }
    }
    for (sha, name) in blobs {
        if let Ok(text) = git(&["cat-file", "blob", &sha]) {
            taken.add(&name, &text);
        }
    }
    Ok(taken)
}

/// The numbers in "N" or "N-N" (a second "D" is allowed: "D217-D261").
fn range(value: &str) -> Vec<u32> {
    let (first, last) = value.split_once('-').unwrap_or((value, value));
    match (
        first.trim().parse::<u32>(),
        last.trim().trim_start_matches('D').parse::<u32>(),
    ) {
        (Ok(first), Ok(last)) if first <= last && last - first < 1000 => {
            (first..=last).collect()
        }
        _ => Vec::new(),
    }
}

/// The number in `NNNN-slug.md`.
fn adr_number(file_name: &str) -> Option<u32> {
    let (number, rest) = file_name.split_once('-')?;
    let markdown = Path::new(rest)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"));
    if number.len() == 4 && markdown {
        number.parse().ok()
    } else {
        None
    }
}

/// The D numbers on a record's `- Decisions:` line.
fn decision_numbers(text: &str) -> Vec<u32> {
    let Some(line) = text.lines().find(|line| line.starts_with("- Decisions:"))
    else {
        return Vec::new();
    };
    line.split(|c: char| !c.is_ascii_alphanumeric())
        .filter_map(|word| word.strip_prefix('D'))
        .filter_map(|digits| digits.parse().ok())
        .collect()
}

fn is_slug(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A ULID-shaped id like the other records': 48 bits of milliseconds,
/// then 80 bits mixed from the clock and the process id.
fn record_id() -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let millis = (nanos / 1_000_000) & ((1 << 48) - 1);
    let noise = (nanos ^ (u128::from(std::process::id()) << 64))
        .wrapping_mul(0x9E37_79B9_7F4A_7C15_F39C_C060_5CED_C835)
        & ((1 << 80) - 1);
    let mut value = (millis << 80) | noise;
    let mut id = String::with_capacity(26);
    for _ in 0..26 {
        let index = usize::try_from(value % 32).unwrap_or(0);
        id.push(char::from(ALPHABET.get(index).copied().unwrap_or(b'0')));
        value /= 32;
    }
    id.chars().rev().collect()
}

fn today() -> Result<String, String> {
    let output = Command::new("date")
        .arg("+%Y-%m-%d")
        .output()
        .map_err(|error| format!("could not run date: {error}"))?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn git(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .map_err(|error| format!("could not run git: {error}"))?;
    if !output.status.success() {
        return Err(format!("git {} failed", args.join(" ")));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

fn say(message: &str) {
    #[expect(clippy::print_stdout, reason = "xtask reports to the terminal")]
    {
        println!("{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_decisions_with_and_without_backticks() {
        assert_eq!(
            decision_numbers(
                "x\n- Decisions: `D138`, D139, D165\n- Issue: #1\n"
            ),
            vec![138, 139, 165]
        );
        assert!(decision_numbers("- Issue: #1\n").is_empty());
    }

    #[test]
    fn next_numbers_follow_the_highest_taken() {
        let mut taken = Taken::default();
        taken.add("0000-template.md", "- Decisions: `DNN`\n");
        taken.add("0048-local.md", "- Decisions: D202, D203, D204\n");
        taken.add("0043-e2e.md", "- Decisions: D210\n");
        taken.add("README.md", "- Decisions: D999\n");
        assert_eq!((taken.next_adr(), taken.next_decision()), (49, 211));
    }

    #[test]
    fn reservations_count_as_taken() {
        let mut taken = Taken::default();
        taken.add("0049-x.md", "- Decisions: D214, D215, D216\n");
        taken.reserve(
            "# note = \"D999\"\n[[reservation]]\ndecisions = \"D217-D261\"\n\
             holder = \"16 threads\"\n[[reservation]]\nadrs = \"0050-0051\"\n",
        );
        assert_eq!((taken.next_adr(), taken.next_decision()), (52, 262));
    }

    #[test]
    fn fills_the_template() {
        let template = "id: 0\ntitle: \"x\"\n- Date: `YYYY`\n\
                        - Decisions: `DNN`\n- Issue: `#N`\n## Decisions\n\n\
                        ### `DNN`: the decision\n\n- a\n";
        let decisions = vec!["D214".to_owned(), "D215".to_owned()];
        let text =
            fill_template(template, 49, &decisions, Some("500"), "2026-10-29");
        assert!(text.contains("title: \"ADR 0049: Short decision title\""));
        assert!(text.contains("- Decisions: D214, D215\n"));
        assert!(text.contains("- Issue: #500\n"));
        assert!(text.contains("### D214: the decision"));
        assert!(text.contains("### D215: the decision"));
        assert!(text.contains("- Date: 2026-10-29\n"));
    }

    #[test]
    fn slugs_are_lowercase_words() {
        assert!(is_slug("faster-fast-suite"));
        assert!(!is_slug("Faster"));
        assert!(!is_slug("--issue"));
    }
}
