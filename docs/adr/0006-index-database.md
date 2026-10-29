---
id: 01M63ANG98YS23FX4RF36Z1FFA
title: "ADR 0006: The index database"
status: verified
owner: "@cruisinme30"
tags: [adr, index, foundation]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D46, D47, D48, D142
- Issue: #56, #230
- Pull requests: #24, #299

:::

## Context

The brief makes files the source of truth and the database a rebuildable
index (`vellum reindex`), on SQLite (SQLCipher) in the single binary and
Postgres in the HA profiles, with `space_id` as the shard key. The crate
`vellum-db` needs one schema that both backends carry and a clear line
between what a reindex rebuilds and what only the database holds.

## Decisions

### D46: one schema, two dialects, forward-only

- Migrations live in `migrations/sqlite` and `migrations/postgres`, one
  file per step with the same number and the same tables. A test opens
  both backends and compares their table lists.
- IDs are `vellum-kernel` IDs: `uuid` in Postgres, a 16-byte blob in
  SQLite. Timestamps are UTC epoch milliseconds (`BIGINT` / `INTEGER`),
  so queries read the same on both.
- Every table carries `space_id`; primary keys and secondary indexes lead
  with it, so a space can move to its own shard without a schema change.
- Migrations are forward-only. An edited migration fails `Index::open`.

### D47: SQLCipher is compiled in, keyed with a raw key

- `libsqlite3-sys` builds SQLCipher with vendored OpenSSL, so the binary
  needs no system crypto library.
- The caller passes a 256-bit key it unwrapped from the KEK; it is set
  with `PRAGMA key = "x'…'"`, which skips SQLCipher's passphrase
  derivation. The key never reaches logs or traces.
- With no key the index is plain SQLite. That is only for an operator
  who chose plaintext mode. A wrong or missing key is `Error::Locked`.

### D48: what a reindex owns

- **Derived from files, rebuilt by `vellum reindex`:** spaces, folders,
  documents and the `page_ancestors` closure.
- **Owned by the database, never deleted by a reindex:** `grants` (with a
  YAML copy in `.vellum-folder.yaml`) and `user_doc_state` (stars,
  recents). `effective_access` is recomputed from them by the
  authorization service.
- A reindex reads files and never writes them. A file or folder without
  an ID is reported, not given one; assigning IDs is the store's job.
- Two files with the same ID: the first by path keeps it, the rest are
  reported as duplicates.

### D142: spaces map to shards in the main database

- With `VELLUM_SHARDS` set, the main database (`VELLUM_DATABASE_URL`)
  holds `space_shards`: one row per space naming its shard. Each shard
  (`VELLUM_SHARD_<NAME>_DATABASE_URL`) carries the full schema and only
  its own spaces' rows.
- A space with no row is on the home shard, the first in the list. A
  reindex records new spaces there, so the map is complete afterwards.
- Moving a space is two steps: update the map, then reindex. Each shard
  commits on its own; a failed run leaves some shards new and some old,
  and the next run converges. An entry naming a removed shard stops the
  reindex before anything is written.

## Alternatives

- **The `sqlx` `Any` driver over both backends**: one code path, but it loses
  `uuid` and typed binds and hides dialect differences. Not chosen; the
  index keeps one pool per backend behind the same `Index` handle.
- **IDs as ULID text**: readable in SQL, but 26 bytes per key against 16
  and no native `uuid` in Postgres. Not chosen.
- **Reindex writes missing IDs into files**: self-healing, but a rebuild
  would then change the source of truth. Not chosen; the store and the
  watcher (ADR 0026) assign IDs.

- **Hash the space ID to pick a shard**: no map to keep, but adding a
  shard moves most spaces. Not chosen; the map moves only what an
  operator asks for.

## Consequences

- Each schema change is written twice; the parity test catches drift.
- Losing the index file loses stars and recents but no content; grants
  come back from the YAML copies.
- Sharded reads look up the map first; callers should cache it per space.
