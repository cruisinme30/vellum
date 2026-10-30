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
and Markdown had no limit. Both formatters also leave long strings and
comments alone, so many lines ran far past any limit. The maintainer asked
for one 80-character limit across every file, enforced, so code reads the same
in a terminal, a side-by-side diff and a review on a phone.

## Decisions

### D320: every line fits in 80 characters

- rustfmt `max_width`, Prettier `printWidth` (web and desktop),
  markdownlint MD013 and `.editorconfig` are set to 80. yamllint's own
  length rule is off: it cannot make the exemptions below, so the check
  holds YAML to 80 like every other file.
- `scripts/ci/check-line-length.sh` fails on longer lines. The pre-commit
  hook runs it on the lines a commit adds; without arguments it checks the
  whole tree.
- Exempt are only lines that can't be wrapped, listed in the script:
  one word; a line that fits once its URLs and pinned hashes are removed;
  Markdown headings, tables, code examples, fence openers and one-line
  fields (also in Rust doc comments); JSON lines holding one string; ESLint
  next-line directives and one-name imports, which Prettier keeps on one
  line; binary files; and files marked `line-length=off` in
  `.gitattributes` (lockfiles, generated files, licence texts, test data
  whose exact bytes are compared, and Fluent messages, whose line breaks
  are part of the text).
- Long strings are split without changing their value: a `\` line
  continuation or `concat!` in Rust, `+` in TypeScript, folded blocks in
  YAML, multi-line strings in TOML.
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

- The formatters left about 2,000 old lines too long. A second pass
  wrapped all but about 80 in Rust (test data strings, signatures rustfmt
  keeps on one line); those need rustfmt to check the result. Once they
  are done, the Fast suite runs the whole-tree check so CI enforces the
  limit too; until then the commit hook guards every new line.
- Open branches conflict with the reformat. They rebase by running
  rustfmt and Prettier, then fixing what the check reports.
- Code formatted at 100 columns now wraps more; generated Rust and
  TypeScript go through the same formatters, so they stay in step.
- `cargo xtask install-hooks` sets `blame.ignoreRevsFile`, so local
  `git blame` skips the reformat as GitHub does.
