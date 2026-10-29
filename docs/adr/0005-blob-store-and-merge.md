---
id: 01M63AVYP292H5BFPDY1WBWG6W
title: "ADR 0005: Blob store port, conditional writes and three-way merge"
status: verified
owner: "@cruisinme30"
tags: [adr, storage, foundation]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D43, D44, D45

:::

## Context

Every Vellum object is a file, and the brief (section 6, D12) says
external edits are merged three-way and never resolved by
last-write-wins. Files live behind one storage trait with adapters for a
local folder, S3, Google Drive, Dropbox, `OneDrive` and Git, and
encryption wraps that trait as a decorator. This record fixes the shape
of that trait and the merge rules in `vellum-store`.

## Decisions

### D43: a narrow `BlobStore` port with conditional writes

- Operations: `get`, `head`, `put`, `delete`, `rename` and a streaming
  recursive `list`. The trait is object-safe, so the backend is chosen
  from configuration at run time and decorators stack.
- Every write and delete takes a precondition: unconditional, only if
  absent, or only if the stored version still has a given `ETag`. A writer
  always states the version it started from, so a stale write is refused
  and goes to the merge path instead of overwriting.
- `ETag` values are opaque per adapter. The local folder uses a `SHA-256` content
  hash, which stays exact on filesystems with one-second timestamps.
- Paths are the intersection of what every backend accepts, and the
  local adapter refuses to traverse symbolic links.
- One shared conformance suite proves each adapter and decorator,
  including that of eight writers racing from one version exactly one
  wins.
- Metrics come from one decorator, so each backend reports the same
  series.

### D44: character-level merge for prose, line `diff3` for manifests

- Prose is merged at character level from `diff-match-patch` diffs: each
  side becomes a list of edits against the base, and the merge applies
  both lists when no two edits overlap. Two people editing different
  sentences of one paragraph both keep their work. Edits that overlap,
  or insertions at the same point, are a conflict. Fuzzy patch
  application is not used, because it can place an edit in the wrong
  spot without reporting a failure.
- Manifests (YAML files and a book's `SUMMARY.md`) use line-based `diff3`,
  because a character-level merge of two structural edits can produce a
  well-formed but wrong manifest.
- Binary content merges only when one side left it unchanged.

### D45: conflicts keep the stored version and copy the late writer's

- On a conflict the version already stored stays at its path. The
  writer's version goes to a sibling,
  `Title (conflict 2026-10-29 alice).md`, numbered on collision.
- The copy gets a fresh front-matter `id` from the caller, because
  identity is the ULID and two files must never share one.
- An edit outranks an external delete: saving over a blob that was
  deleted meanwhile recreates it. Trash and history make that safe.
- A save that keeps meeting newer versions gives up after eight merges
  with an error rather than looping.

## Consequences

- Cloud-drive and Git adapters must offer conditional writes (S3
  `If-Match`/`If-None-Match`, Drive revision checks, Git
  compare-and-swap on the ref) and pass the same suite.
- Editors outside Vellum cannot take the local adapter's in-process
  lock. Their edits are found by the watcher and go through the same
  merge.
- Conflict copies are ordinary documents. The UI lists them, and both
  authors are notified.
