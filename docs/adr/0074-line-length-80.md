---
id: 01M66QTP4BTECWS1K12BVGZXED
title: "ADR 0074: Lines stop at 80 characters"
status: draft
owner: "@cruisinme30"
tags: [adr, tooling, style]
vellum: 1
---

::: facts

- Date: 2026-10-30
- Status: accepted
- Decisions: D320
- Issue: #564
- Pull requests: to be linked when merged

:::

## Context

Line lengths differed by tool: rustfmt wrapped Rust at 100 columns,
Prettier wrapped the web app and desktop shell at 100, yamllint allowed 120
and Markdown had no limit. Rustfmt and Prettier also leave long strings
and comments alone, so many lines ran far past any limit. Gourav asked for
one 80-character limit across every file, enforced, so code reads the same
in a terminal, a side-by-side diff and a review on a phone.

## Decisions

### D320: every line fits in 80 characters

- rustfmt `max_width`, Prettier `printWidth` (web and desktop) and
  `.editorconfig` are set to 80; yamllint `line-length` and markdownlint
  MD013 follow once the old lines they would flag are wrapped.
- `scripts/ci/check-line-length.sh` fails on longer lines. The pre-commit
  hook runs it on the lines a commit adds; without arguments it checks the
  whole tree.
- Exempt are only lines that can't be wrapped: a line that fits once its
  URLs are removed, Markdown table rows, binary files, and files marked
  `line-length=off` in `.gitattributes` (lockfiles, generated files,
  licence texts, test data whose exact bytes are compared, and Fluent
  messages, whose line breaks are part of the text).
- Long strings are split without changing their value: a `\` line
  continuation or `concat!` in Rust, `+` in TypeScript.
- Size limits counted lines at 100 columns, so they grow by the same
  quarter: ESLint `max-lines` from 400 to 500, clippy's
  `too-many-lines-threshold` from 100 to 125.
- The repo was reformatted in one commit, listed in
  `.git-blame-ignore-revs` so `git blame` skips it.

## Alternatives

- **Keep 100 columns**: matches the old setup, but is not what was asked.
- **Formatters only, no check**: strings and comments would stay long, and
  rustfmt gives up silently on lines it can't fit.
- **Exempt strings and comments**: most of the long lines are exactly
  those, so the limit would mean little.

## Consequences

- rustfmt and Prettier could not wrap every old line: long strings and
  macros, shell scripts, workflows, site HTML and CSS, and config files
  still hold about 2,000. A follow-up wraps them, sets yamllint and
  markdownlint to 80, and has the Fast suite run the whole-tree check so
  CI enforces the limit too.
- Open branches conflict with the reformat. They rebase by running
  rustfmt and Prettier, then fixing what the check reports.
- Code formatted at 100 columns now wraps more; generated Rust and
  TypeScript go through the same formatters, so they stay in step.
- `cargo xtask install-hooks` sets `blame.ignoreRevsFile`, so local
  `git blame` skips the reformat as GitHub does.
