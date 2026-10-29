---
id: 01M6450PG658KEX8XYM2RVF0RE
title: "ADR 0037: Running several replicas"
status: draft
owner: "@cruisinme30"
tags: [adr, deploy, scale]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D150, D151, D159, D160
- Issue: #432, #433, #434, #435, #437
- Pull requests: none yet (delivered as a bundle)

:::

## Context

The Organisation profile runs "two of everything" with no single point of
failure, and the Helm chart starts two or more replicas of the same binary
behind one Service. A review of `develop` (#432) found that several
services keep state in process memory with no way to learn what another
replica changed. This record collects the decisions that make each service
safe to run as several replicas. Each fix adds its decision here.

## Decisions

### `D150`: Work commits through a journal with a conditional claim

- Each replica keeps the Work engine in memory, as before. A small blob,
  `.vellum/work/journal.json`, numbers every command and lists the files
  it changed (the last 512 commands).
- A command first loads what other replicas committed, then applies
  itself in memory, then claims the next revision with an `If-Match`
  write. A replica that loses the claim restores its state and runs the
  command again, so two replicas never hand out the same `ENG-n`.
- The claim stays `pending` only while the command's files are written,
  with a 30-second lease. Writers wait for it; readers stop one revision
  short. A crashed writer's lease expires, and what it wrote is loaded.
- Reads check the journal at most every 250 ms, so another replica's
  write shows within a second, as the brief asks.
- Changed issues and comment threads load one by one; a changed team,
  cycle or project rebuilds Work from the files. A replica more than 512
  commands behind rebuilds too.

### `D151`: replicas share a heartbeat table; collab forwards to the room owner

- Each replica beats every 2 s in `cluster_nodes` (index database,
  migration 0007); a replica silent for 6 s is gone. `cluster_leases` in
  the same migration names work one replica does at a time.
- The owner of a room is `placement::owner` over the live list, the same
  on every replica (rendezvous hashing).
- A replica that receives a connection for a room it does not own
  authorizes the editor, then relays the socket to the owner. The owner
  trusts the join through an HMAC-signed header (`VELLUM_CLUSTER_KEY`, 30 s
  validity) instead of authorizing again.
- When the list changes, rooms a replica no longer owns are drained and
  their editors reconnect to the new owner. An unreachable owner means the
  replica serves the room itself, so re-home costs one reconnect.

### `D159`: the workspace files live in the bucket `VELLUM_BLOB_URL` names

- `vellum serve` opens the S3 adapter when `VELLUM_BLOB_URL` is an `s3://`
  URL, and the data folder when it is unset. Any other value, or missing
  S3 keys, stops startup with a message naming the variable.
- The file watcher runs only for a data folder. A bucket is written only
  through Vellum, so there are no outside edits to follow.
- A `storage` readiness check lists an empty folder in the store, so
  `/readyz` fails while the bucket is unreachable, without logging an
  error on every healthy probe.
- A shared bucket with a per-replica SQLite index logs a warning; the team
  and organisation profiles already require Postgres.
- Multi-replica Work (D150) relies on this: its journal claim needs the S3
  adapter's `If-Match` and `If-None-Match` writes, which the storage
  conformance suite checks are atomic.

### `D160`: sign-in records live in the index database

- With a database URL, accounts, sessions, API tokens, `OAuth` grants,
  pending consents and codes, and lockout counters live in
  `identity_records` (migration 0008); the single binary keeps
  `identity.json`, which the first start with Postgres imports.
- Each replica caches the records. A read first fetches the rows written
  since its cached version, so a sign-out or a disabled account holds on
  the next request to any replica.
- A write bumps `identity_meta.version` under its row lock, runs on fresh
  records, and writes only the changed rows with the new version. Deletes
  leave tombstones, purged after 1024 versions.
- Lockout counters are shared and keyed by a hash of the address, so N
  replicas allow the attempts of one.
- Every replica signs CSRF tokens with one key: the chart makes a
  `<release>-csrf` Secret, compose reads `secrets/csrf-key`.

## Alternatives

- **Work state in Postgres**: the most scalable, but it rewrites the
  engine and makes the single binary depend on the index for Work.
- **One owner replica for Work, others forwarding**: needs replica
  discovery and request forwarding; the collab fix may provide both later.
- **Ingress affinity by document path**: depends on the ingress
  controller and cannot drain rooms when replicas change.
- **A shared volume (`ReadWriteMany`) at `/data`**: keeps the local
  folder code, but few clusters offer one, and file locking over network
  file systems cannot serve as Work's atomic claim.
- **Async SQL per identity call**: no cache, but every sign-in path and
  the in-flight single sign-on work would be rewritten around `await`.
- **Full reload whenever anything changed**: simple, but each change would
  cost every replica a read of every Work file.

## Consequences

- Work needs a blob store whose conditional writes are atomic across
  replicas (S3 and compatible). Two processes on one local folder are not
  supported.
- Each command costs two extra small writes and one read. Work writes stay
  serialised across the workspace; per-team journals can lift that if a
  large workspace needs more than about 20 commands a second.
- Forwarded collab traffic between replicas is plain WebSocket inside the
  cluster network; the chart's network policy limits who can reach it.
- A bucket needs static S3 keys today; the chart's workload-identity
  option refuses to start until the S3 adapter supports it.
- Edits made to Work files outside Vellum are still not picked up while
  running; that is unchanged.
- Sign-in costs one indexed query per request and writes are serialised
  workspace-wide; per-record queries can replace the cache if load tests
  show it. Break-glass codes are still a file on one replica.
