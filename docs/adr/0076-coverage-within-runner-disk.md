---
id: 01M67DYBWTD87AENQ0SEZCC0XX
title: "ADR 0076: Coverage that fits a runner's disk"
status: draft
owner: "@cruisinme30"
tags: [adr, ci, foundation]
vellum: 1
---

::: facts

- Date: 2026-10-30
- Status: accepted
- Decisions: D324
- Issue: #574
- Pull requests: `ci/coverage-disk` (through the integrator)
- Builds on: ADR 0049 (a shorter fast suite)

:::

## Context

The Rust coverage job of the full suite crashed on release pull
request #544 (run 41133469810): `ld` died with a bus error while linking the
`vellum-format` test programs. `cargo` builds one test program per file in
a crate's `tests/` folder, and each one links the crate's whole
dependency graph again, with coverage instrumentation. The workspace had
165 test programs (35 libraries, 1 binary and 129 test files), about
18 GB of build output on a runner with 14 GB free.

## Decisions

### D324: one test program per crate, and four coverage shards

- Each crate's integration tests are modules of one program,
  `tests/it/main.rs`, one module per former file. Shared helpers
  (`support`, `common`, `fakes`) are modules of that program too, and
  fixtures stay in `tests/fixtures`. The workspace now has 69 test
  programs; every test is kept, renamed `<module>::<test>`.
- `cargo xtask boundaries` fails when a crate has more than one
  integration test program, so the count cannot creep back.
- The coverage tests run as four jobs, "Rust tests N/4", each running
  `cargo llvm-cov nextest --no-report --partition hash:N/4` and
  uploading its coverage report. They share one Rust cache.
- The "Rust coverage" job merges the four reports with
  `scripts/ci/merge-coverage.sh`: a line is covered when any shard ran
  it. It fails under 85 % of lines, as before. "Full suite" waits on the
  shards and the merge, so the ruleset on main needs no change.
- Commands that named a test file now name the program and a module
  filter: `cargo test -p vellum-export --test it -- perf::`.

## Alternatives

- **Free space by deleting the runner's preinstalled tools**: hides the
  growth instead of fixing it, and was ruled out with the fix.
- **Shards by crate instead of by test hash**: each runner would build
  less, but four different builds need four caches in the shared 10 GB
  budget, and the crate lists drift out of balance.
- **A larger runner**: costs money on every release and leaves 165
  programs that the fast suite and local builds still pay for.
- **`autotests = false` with one `[[test]]` entry per crate**: the same
  result with more manifest lines; `tests/it/main.rs` is found by `cargo`.

## Consequences

- Fewer links: local builds, sandboxes and the fast suite's Rust test
  job also build about 100 fewer programs.
- Every shard still builds the whole workspace, so a shard's disk use
  is that of one 69-program build. If that grows close to the runner's
  disk again, shard by crate.
- The merged report counts lines the way the `llvm-cov` summary does
  only approximately; the floor may move by a fraction of a percent.
- Tests in one program share a process under `cargo test`. CI runs them
  with nextest, one process per test, so this only matters locally.
