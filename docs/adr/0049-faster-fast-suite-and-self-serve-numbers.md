---
id: 01M64S7RBFT0SFF8HAC9TRT77G
title: "ADR 0049: A shorter fast suite, and decision numbers without waiting"
status: draft
owner: "@cruisinme30"
tags: [adr, ci, foundation]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D214, D215, D216
- Issue: #500
- Pull requests: `ci/faster-fast-suite` (through the integrator)
- Builds on: ADR 0035 (faster pull request checks), ADR 0048 (local checks)

:::

## Context

Since the integrator thread batches the threads' branches, every batch
and every push to develop waits on the fast suite. Across 63 runs from
9 AM to 2:20 PM PT on 2026-10-29, a run took 12.5 minutes (median). Two
jobs set that. The Rust job took 11 minutes, all in sequence: about 2
minutes to restore the cache, then clippy and docs, then tests (6.7
minutes, of which about 5 build some 140 test binaries and 1.6 run 1,408
tests). The UI layout check took 8.6 minutes, running 334 tests on the
runner's default two Playwright workers.

Threads also stopped work to ask the coordinator for ADR and D numbers.

## Decisions

### D214: shard the quick UI layout check

- The quick check runs as two matrix jobs, `--shard=N/2`, with four
  workers each instead of the default two.
- Expected: about 3 minutes instead of 8.6, at the cost of one more job
  per run.
- Only "Fast suite" is a required check, so the renamed jobs need no
  ruleset change.

### D215: run Rust tests in parallel, with less debug info

- "Rust tests" is its own job, started as soon as "Changed areas"
  finishes. "Rust" keeps format, clippy, docs and the policy checks.
- Each job keeps its own Rust cache, saved only on develop (D138).
- `[profile.dev]` uses `debug = "line-tables-only"`, and dependencies
  build with no debug info. Panic traces keep file and line numbers, while
  linking, cache restores and sandbox target directories all get smaller.
- The full suite still runs the tests once, in its coverage job.

### D216: threads pick their own ADR and D numbers

- `cargo xtask adr next` prints the next free numbers. It reads
  `docs/adr/` in the working tree and on every branch on `origin`.
- `cargo xtask adr new <slug> --decisions N --issue N` starts a record
  from the template with those numbers. Pushing the branch claims them.
- `docs/adr/reserved.toml` lists numbers the coordinator handed out
  before any branch holds them. The tool counts them as taken, and the
  check fails when two reservations overlap.
- `scripts/ci/check-adrs.sh` fails when two records share an ADR number
  or a D number, so collisions between branches fail the integration
  batch before they reach develop.

## Alternatives

- **A shared compiler cache (`sccache`)**: the 10 GB cache limit is
  already shared with rust-cache. Revisit if builds still dominate.
- **Testing only changed crates in CI**: integrator batches touch most
  crates anyway, and develop would then need its own full run.
- **A larger runner**: needs a paid plan or an organization.
- **Every thread records its numbers in one file**: every new record
  would edit the same file, so merges would conflict. Only the
  coordinator's rare bulk reservations go in `reserved.toml`.

## Consequences

- A fast-suite run should drop from about 12.5 to about 7 minutes. The
  critical path becomes the Rust tests job.
- Each run uses one more UI job and one more Rust job. Runner minutes
  rise by about 3 per run, and the queue rises a little at peaks.
- Two threads that pick numbers before either pushes still collide. CI
  catches it, and the later branch takes `cargo xtask adr next` again.
