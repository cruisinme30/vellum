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
- Decisions: `D138`, `D139`, `D140`, `D165`, `D166`, `D167`
- Issue: `#264`, `#446`
- Pull requests: `chore/ci-speed`, `chore/ci-dedupe-tests`,
  `chore/release-ci-speed`

:::

## Context

About 25 threads open pull requests into develop at once and merge them
with auto-merge, on free GitHub-hosted runners. Measured on 2026-10-29: the
Fast suite takes about 3 minutes with a warm cache, but CodeQL's Rust
analysis takes about 6 minutes plus 1 to 2 minutes in the queue. Once the
develop ruleset required CodeQL results, every merge waited 8 to 10
minutes. The repository ran about 100 workflow runs every 15 minutes, so
jobs also queued for runners.

The release check had its own problem. Full suite run 40903463496 (release
pull request #443) used about 190 runner minutes in its first 35 minutes
and had not finished. Its 12 browser jobs took 22 to 31 minutes each,
about 25,500 UI checks in total: every one of the ~135 pages of the help
books and decision records, in 15 display modes, 3 browsers and 4 screen
sizes. The two from-scratch release builds ran one after the other.

## Decisions

### `D138`: keep the Rust cache warm and stop hung tests early

- Only pushes to develop save the Rust cache; pull requests restore it.
- Twenty-five pull request caches would otherwise fill the 10 GB limit and
  evict the develop cache, turning a 2-minute job into a cold build.
- `.config/nextest.toml` has a `ci` profile that kills a test after
  2 minutes, so a hang fails fast instead of holding the job for 30.

### `D139`: run only the jobs a change needs

- `scripts/ci/changed-areas.sh` maps changed paths to the Rust, Web and
  Desktop jobs. Pull requests into develop skip Web unless `web/` changed,
  Desktop unless `desktop/` or `web/` changed, and Rust when only ADRs,
  the site or top-level Markdown changed.
- Help books are compiled into `vellum-help`, so they count as Rust.
- A change to workflows or `scripts/ci/` runs everything, as do pushes,
  the merge queue and the full suite.
- `cargo xtask fast` uses the same script, so the pre-push hook skips
  what CI skips.
- Editing a pull request's title or description reruns only the
  description checks when the same commit already passed the Fast suite.
- The required check names (Fast suite, Labels, Full suite) stay the same.

### `D140`: CodeQL gates main, not develop

- The develop ruleset drops its `code_scanning` rule; main keeps it.
- CodeQL scans release pull requests, pushes to main and develop weekly
  (`.github/workflows/codeql.yml`), replacing the CodeQL default setup of
  [ADR 0032](0032-repository-security-controls.md). A high or critical
  alert blocks the release into main.
- Merges into develop wait only for the Fast suite and Labels.
- The CodeQL default setup was switched off in the repository settings
  on 2026-10-29, so `codeql.yml` alone scans and gates main; it can still
  be started by hand.

### `D165`: help pages that share a template get the default mode only

- In the full suite, every app route, the landing page, the docs home and
  each book's first and print pages get every check of
  [ADR 0031](0031-ui-quality-suite.md): all display modes, loading and
  keyboard focus.
- Every other help page is a "sweep" page: it gets the layout audit, axe
  and the error checks in the default mode only, in every browser and size.
- Why: the chapters share one mdBook template, and a book's print page
  lays out every chapter's content, so the other modes repeated the same
  checks page by page. Checks per browser drop from 8,520 to 1,320.
- `UI_SCOPE=exhaustive` (a choice when starting the Full suite by hand)
  still runs every page in every mode.

### `D166`: the two reproducible builds run side by side

- Two jobs each build the release binary from scratch and upload it; the
  image job compares them byte for byte, builds the image and checks the
  budgets. The release check waits for one build instead of two.
- The builds now run on different machines, which makes the
  reproducibility check stricter, not weaker.

### `D167`: two browser shards, inside the concurrency limit

- The browser matrix is 3 browsers times 2 shards, about 10 minutes each.
- Six jobs instead of twelve keep the Full suite under the account's
  20 concurrent jobs, so no browser job waits for a runner.

## Alternatives

- **Split the Rust job into parallel jobs**: each job rebuilds the
  workspace, and runners are the scarce resource, so it would queue more.
- **`sccache`**: rust-cache already restores a warm target folder; `sccache`
  adds a service for little gain.
- **CodeQL advanced setup with path filters**: a skipped analysis leaves
  the code scanning gate waiting for results, which blocks merges.
- **Keep CodeQL on develop with the default suite**: still about
  6 minutes for Rust.

- **More workers per browser job**: the loading checks time first paint
  on a throttled network, and a busier runner would make them flaky.
- **Warm the coverage cache from develop**: coverage is not on the release
  check's longest path (about 11 minutes against about 19 for the builds).

## Consequences

- Most pull requests merge in about 3 minutes instead of 8 to 10.
- A high CodeQL alert can reach develop; it must be fixed before the next
  release, since main stays gated.
- A new top-level folder counts as Rust until `changed-areas.sh` lists it.
- Revisit if the repository moves to an organisation with a merge queue.
- The full suite runs each test once: Rust tests only with coverage, UI
  tests only in the browser matrix, and nothing again on the push to main.
- A release pull request opened from develop itself fails fast; releases
  come from a frozen `chore/release-…` branch.

From 2026-10-29, develop is pushed about every two hours after the full
suite passes locally. A push to develop runs the Fast suite once for the
areas it changed, the Full suite runs only on release pull requests, the
nightly jobs skip unchanged nights, and no run cancels another.

The release check (D165-D167) aims at about 20 minutes of wall time and
about 140 runner minutes, down from more than 40 minutes and about 420. A
layout bug that shows only on one help chapter in a non-default mode is
caught by the exhaustive run, not by the release check.
