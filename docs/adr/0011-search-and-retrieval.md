---
id: 01M63DAYVFY7HHZRQ8VTBBZEJW
title: "ADR 0011: Search and retrieval"
status: draft
owner: "@cruisinme30"
tags: [adr, search, engine]
vellum: 1
---

::: facts

- Date: `2026-10-29`
- Status: accepted
- Decisions: `D61`, `D62`, `D63`
- Issue: `#102`, also `#99`, `#100`, `#172` and `#403`
- Pull requests: `#236`, and the page feed after it

:::

## Context

The brief fixes the engines (D23): embedded tantivy in the single binary,
Postgres full-text search with `pg_trgm` in HA, and Meilisearch as an
optional adapter, all behind one trait. It also fixes what search returns:
block-level hits with IDs and citations, the heading section as context,
BM25 plus optional local ONNX embeddings fused by reciprocal rank, drive
filters, permission filtering at query time, incremental indexing on save
and rebuilds from the files (R-SR-1 to R-SR-5, R-DR-9, R-AI-4, R-AI-7).
The index is encrypted like everything else (D24).

The `vellum-search` crate is a service-layer crate. The `SearchBackend`
trait is the one port; `SearchService` adds scoping, fusion, the
permission check and snippets once for every engine; a conformance suite
runs the same checks against each engine.

## Decisions

### `D61`: permissions are checked at query time, never stored in the index

- The index stores no ACLs. A query first narrows to the spaces the
  principal can read (`ReadAccess::readable_spaces`), then every candidate
  document passes one exact batch check (`ReadAccess::filter_readable`,
  backed by `vellum-authz` and its `effective_access` table). Candidates
  widen until a page is full, so filtering never shortens a page early.
- Unknown principals read nothing. Requested spaces only ever narrow the
  readable set. Context lookups answer "not found" for unreadable blocks
  so existence does not leak.

### `D62`: blocks are retrieved, sections give context, ranks are fused

- The retrieval unit is a block; the context unit is the heading section
  it sits in. Hits carry `vellum://doc/{doc}#{block}` citations.
- Lexical ranking is the engine's own (BM25 in tantivy, `ts_rank_cd` in
  Postgres). Query words are OR-ed, so more matched words rank higher.
- With an embedding model, the nearest blocks by cosine similarity (floor
  0.35) form a second list. The two lists are fused by reciprocal rank
  with `k = 60` and equal weights. A failed query embedding degrades to
  lexical search instead of failing.
- Highlights come from one shared analyzer, so every engine marks the
  same words.

### `D63`: the local index is encrypted chunk by chunk

- Every tantivy file is sealed in 4 `KiB` chunks with XChaCha20-Poly1305
  under an index key from the key hierarchy. The file name, chunk number
  and last-chunk flag are bound as associated data, so swapping,
  reordering or truncating chunks fails. Reads decrypt only the chunks a
  range touches.
- Plaintext mode is allowed only when every space is plaintext. Postgres
  relies on TDE or disk encryption, as D24 states for HA.
- A stamp file records the layout version and the embedding model. A
  mismatch, or a key that cannot read the stamp, empties the index and
  asks for a rebuild instead of failing.

## Alternatives

- **ACLs in the index**: every grant change re-indexes blocks, missing the one-second target.
- **Post-filter only**: ranks every space even when the caller reads one.
- **External vector database**: ruled out by the brief.
- **ONNX Runtime**: a native library per platform; tract is pure Rust.

## Consequences

- Grant changes reach search immediately, at the cost of one batch
  authorization call per query.
- The three engines rank differently in detail; the conformance suite
  fixes the behaviour that must not differ.
- Embeddings are optional and exact (a filtered scan). A model change
  rebuilds the index. If a shard outgrows exact scans, the Meilisearch
  adapter is the path, as D23 says.
- Postgres search tables live in their own `vellum_search` schema with
  their own migrations.
- The index learns about saved pages by sweeping the drive's page rows
  about once a second and comparing fingerprints, because the drive emits
  no events and its rescanning records no activity. A sweep reads one row per
  page, so very large drives may want a longer interval.
