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
- Decisions: D150, D151
- Issue: #432, #435, #437
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

## Alternatives

- **Work state in Postgres**: the most scalable, but it rewrites the
  engine and makes the single binary depend on the index for Work.
- **One owner replica for Work, others forwarding**: needs replica
  discovery and request forwarding; the collab fix may provide both later.
- **Ingress affinity by document path**: depends on the ingress
  controller and cannot drain rooms when replicas change.
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
- Edits made to Work files outside Vellum are still not picked up while
  running; that is unchanged.
