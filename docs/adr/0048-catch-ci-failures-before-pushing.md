---
id: 01M64FWHHMZ126N2YJTQRCZ10C
title: "ADR 0048: Catch CI failures before pushing"
status: draft
owner: "@cruisinme30"
tags: [adr, ci, tooling]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D202, D203, D204
- Issue: #490
- Pull requests: #494

:::

## Context

Pull requests kept failing CI after local checks passed. An audit of 40
failed runs on 2026-10-29 (#490) found 42 failures. Nineteen came from the
pull request's title, branch, description or issue link, which only CI
checked. Six came from linters that cloud sessions did not have, so
`cargo xtask fast` skipped them and still passed. Four were Rust tests or
clippy, which threads skip locally to save time and disk. The local checks
have to catch these without running the full suite, which CI already runs
([ADR 0035](0035-ci-speed.md)).

## Decisions

### `D202`: cloud sessions install the Fast suite's tools

- `scripts/dev/install-tools.sh` installs every tool `cargo xtask fast`
  calls, at the versions CI uses, into `~/.local/bin`.
- A tool already on `PATH` is left alone, so a second run is instant; the
  first takes about 30 seconds.
- The cloud environment's setup script runs it, piped from `develop`, so
  every new session starts with the tools.
- Helm stays CI-only: its download host is not on the cloud allowlist.

### `D203`: a missing tool fails `cargo xtask fast`

- A step whose tool is missing now fails the run, and the summary says how
  to install it. `VELLUM_ALLOW_MISSING_TOOLS=1` restores the old skip.
- The Rust tests run only for the workspace crates the branch changed, so
  a push tests its own code without the full suite.
  `VELLUM_FAST_ALL_TESTS=1` tests every crate.

### `D204`: `cargo xtask pr-check` runs the pull request checks first

- Takes the title and a file holding the description, and runs CI's own
  scripts: branch and title rules, the description template, and the
  linked issue with its decision record.
- Looks the issue up with `gh`, anonymously when the session's token does
  not work; with no GitHub access the issue is left to CI and it says so.
- Run it before opening or editing a pull request.

## Alternatives

- **Run the whole Fast suite, tests included, before every push**: rejected;
  it takes long and fills the cloud sandbox's disk, and CI repeats it.
- **A session-start hook committed to the repository**: rejected; setting
  up the environment belongs to its setup script, not to repository code
  that runs automatically.
- **Check the pull request text in a git hook**: a push carries no title or
  description, so the check needs its own command.

## Consequences

- Linter and description failures now show up before the push, where they
  cost seconds rather than a CI cycle.
- A test in a crate that the branch did not touch, but whose behavior the
  change affects, still fails only in CI.
- The tool versions live in two places, `ci.yml` and the install script;
  a bump updates both.
