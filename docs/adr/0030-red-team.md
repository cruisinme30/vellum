---
id: 01M63F2XJBKCW3B0QSAXBQ10G4
title: "ADR 0030: Red team, hostile-input limits and attack suites"
status: draft
owner: "@cruisinme30"
tags: [adr, security, testing]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D120, D121, D122
- Issue: #259
- Pull requests: #265

:::

## Context

Every Vellum file is untrusted input: a collaborator, a synced folder, an
import or an MCP client can write any bytes, and the server parses them on
save, render and index. The brief (section 11 and 12) asks for a hardened,
self-hosted service that meets its latency and memory budgets under load.
The project lead asked for continuous red teaming: attack what has
merged, prove each finding, and make sure the fix lands (#259). The first sweep found a 3 KB
document that aborts the process with a stack overflow (#260).

## Decisions

### D120: bounded work for every hostile input

- Code that walks untrusted, nested input either recurses to a fixed,
  documented depth or iterates. A stack overflow aborts the whole process,
  so it is treated as a remote crash, not a bug.
- The Markdown parser caps nesting at `vellum_format::MAX_NESTING` (32)
  levels of blocks and, separately, of inline markup. Deeper content is
  kept as plain text with a `NestingTooDeep` diagnostic; nothing is lost
  silently and the result still round-trips.
- Parsers also bound total work: a library feature that is quadratic in
  some input (comrak's alert check on a run of `>`) is fenced off before
  the library sees it. Front matter uses a tight serde-saphyr budget
  (depth 16, 20 000 nodes, 1 000 aliases).
- Tests run hostile cases on a 1 MiB stack in a debug build, half the
  smallest runtime default, so a regression fails loudly in CI.

### D121: attack suites live in the repository

- Each crate that reads untrusted input has `tests/hostile.rs` (or
  `tests/hostile_*.rs`): small, deterministic attack cases that run in the
  fast suite on every pull request.
- Coverage-guided fuzz targets live in `fuzz/` (cargo-fuzz, nightly
  toolchain) and run in the nightly full suite with a time budget; a crash
  becomes a `hostile.rs` case before it is fixed.
- Load and chaos tests stay with the deploy thread (#193, #194); the red
  team adds scenarios there rather than a second harness.

### D122: one issue per finding, fixed with a regression test

- Every finding gets its own issue with a repro, a severity (critical,
  high, medium, low, info) and the labels `security` or `bug` plus
  `area/*`. #259 lists them all.
- The red team either lands the fix with a failing-first regression test,
  or hands the issue to the owning thread and verifies the fix.
- Accepted risks are recorded on their issue and in the findings page,
  with the decision that accepts them.

## Alternatives

- **Grow the thread stack** (`stacker`, bigger tokio stacks): moves the
  limit without removing it, and costs memory on every worker.
- **Reject deep documents outright**: loses the user's content; showing
  the deep part as text keeps the file readable and editable.
- **Fuzzing only, no hand-written cases**: fuzz targets find the crash but not
  the quadratic cases quickly, and they do not run on every pull request.

## Consequences

- Documents nested deeper than 32 levels render their deep part as text.
  No real document is expected to reach that; the limit can be raised if
  one does.
- New parsers (formula language, CRDT updates, MCP messages, imports) must
  come with a `hostile.rs` before they merge; the red team reviews them.
- The fast suite grows by a few seconds per crate; heavy cases belong in
  the nightly fuzz job instead.
