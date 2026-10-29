---
id: 01M64C0Z9R7V3K8T2Q5W1N6HXD
title: "ADR 0040: Sharding and shared limits in serve"
status: draft
owner: "@cruisinme30"
tags: [adr, deploy, scale, security]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D173
- Issue: #440, #436, #432
- Pull requests: #463

:::

## Context

With several replicas behind one Service ([ADR 0037](0037-running-several-replicas.md)),
the gateway's rate limiter lived in each replica's memory, so a caller
spread over N replicas got N times the configured allowance (#440). Live
editing makes it worse the other way round: a replica that forwards
editors to the room's owner sends all of them from its one address. This
record covers the limits and, as they land, the shard routing in serve
(#436) that the same umbrella asks for (#432).

## Decisions

### `D173`: rate allowances are counted per minute across the cluster

- Each replica keeps its own GCRA limiter as the first line: it caps
  bursts at once, without a database round trip.
- On top, each replica counts requests per caller (the user, or the
  address when signed out) in one-minute windows, and adds its counts to
  `rate_counts` in the index database once a second. The upsert returns
  the cluster's total, so a caller over the per-minute allowance is
  refused on every replica, at most about a second late.
- The allowance per minute is the configured rate per second times 60
  plus the burst, the same budget one replica already allowed.
- Requests carrying a live-editing join signed with the cluster key, for
  the room their path names and not expired, skip both limits.
- If the database cannot be reached, counts are dropped and each replica
  falls back to its own limiter. Windows older than two minutes are pruned
  by the replica holding the maintenance lease.

## Alternatives

- **Redis counters**: exact and fast, but one more service to run for
  the HA profiles, when the index database already holds shared state.
- **A database write per request**: exact, but puts a round trip on
  every request and a hot row per busy caller.
- **Dividing the allowance by the number of replicas**: needs no shared
  state, but is wrong whenever traffic is uneven, which it usually is.

## Consequences

- A limit of 100 a minute holds at about 100 across three replicas;
  the overshoot is what the cluster serves in the second before a count
  is shared.
- `rate_counts` holds callers' user IDs and addresses for two minutes,
  inside the index database that is already encrypted at rest on SQLite.
- A stolen cluster key would let its holder skip rate limits on the
  collab path; the key already lets its holder join any room, so it stays
  a secret of the same weight.
- MCP keeps its per-token limiter and in-process change events until the
  MCP server is mounted in serve; it will count through the same table.
