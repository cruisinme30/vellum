---
id: 01M63PFYD56PJ24D4EBCRRMB9T
title: "ADR 0035: Faster pull request checks"
status: draft
owner: "@cruisinme30"
tags: [adr, ci, foundation]
vellum: 1
---

::: facts

- Date: `2026-10-29`
- Status: accepted
- Decisions: `D138`, `D139`, `D140`
- Issue: `#264`
- Pull requests: `chore/ci-speed`

:::

## Context

About 25 threads open pull requests into develop at once and merge them
with auto-merge, on free GitHub-hosted runners. Measured on 2026-10-29: the
Fast suite takes about 3 minutes with a warm cache, but CodeQL's Rust
analysis takes about 6 minutes plus 1 to 2 minutes in the queue. Once the
develop ruleset required CodeQL results, every merge waited 8 to 10
minutes. The repository ran about 100 workflow runs every 15 minutes, so
jobs also queued for runners.

## Decisions

### `D138`: keep the Rust cache warm and stop hung tests early

- Only pushes to develop save the Rust cache; pull requests restore it.
- Twenty-five pull request caches would otherwise fill the 10 GB limit and
  evict the develop cache, turning a 2-minute job into a cold build.
- `.config/nextest.toml` has a `ci` profile that kills a test after
  2 minutes, so a hang fails fast instead of holding the job for 30.

### `D139`: run only the jobs a change needs

- `scripts/ci/changed-areas.sh` maps changed paths to the Rust and Web
  jobs. Pull requests into develop skip Web unless `web/` changed, and
  skip Rust when only ADRs, the site or top-level Markdown changed.
- Help books are compiled into `vellum-help`, so they count as Rust.
- A change to workflows or `scripts/ci/` runs everything, as do pushes,
  the merge queue and the full suite.
- `cargo xtask fast` uses the same script, so the pre-push hook skips
  what CI skips.
- The required check names (Fast suite, Labels, Full suite) stay the same.

### `D140`: CodeQL gates main, not develop

- The develop ruleset drops its `code_scanning` rule; main keeps it.
- CodeQL still scans every pull request and push and reports alerts, and a
  high or critical alert blocks the release into main.
- Merges into develop wait only for the Fast suite and Labels.

## Alternatives

- **Split the Rust job into parallel jobs**: each job rebuilds the
  workspace, and runners are the scarce resource, so it would queue more.
- **`sccache`**: rust-cache already restores a warm target folder; `sccache`
  adds a service for little gain.
- **CodeQL advanced setup with path filters**: a skipped analysis leaves
  the code scanning gate waiting for results, which blocks merges.
- **Keep CodeQL on develop with the default suite**: still about
  6 minutes for Rust.

## Consequences

- Most pull requests merge in about 3 minutes instead of 8 to 10.
- A high CodeQL alert can reach develop; it must be fixed before the next
  release, since main stays gated.
- A new top-level folder counts as Rust until `changed-areas.sh` lists it.
- Revisit if the repository moves to an organisation with a merge queue.
