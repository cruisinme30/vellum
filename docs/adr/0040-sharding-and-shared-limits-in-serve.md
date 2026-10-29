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
- Decisions: D173, D174, D175
- Issue: #440, #436, #432
- Pull requests: #463, shard routing to follow in its own pull request

:::

## Context

With several replicas behind one Service ([ADR 0037](0037-running-several-replicas.md)),
the gateway's rate limiter lived in each replica's memory, so a caller
spread over N replicas got N times the configured allowance (#440). Live
editing makes it worse the other way round: a replica that forwards
editors to the room's owner sends all of them from its one address. This
record covers the limits and the shard routing in serve (#436) that the
same umbrella asks for (#432): the chart could already run one database
per shard, but `vellum serve` put every space on the first.

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

### `D174`: serve routes each space to its shard

- With `VELLUM_SHARDS` set, serve opens every shard. The drive index,
  the permission store and search route each call naming a space to that
  space's shard; a call naming only a node finds the node's shard first.
- Lists across spaces (every space, stars, recents, backlinks, the spaces
  a person can reach, search filters) ask every shard and merge.
- The space-to-shard map in the main database is kept in memory. A new
  space goes to the shard holding the fewest spaces, recorded on its first
  write; when two replicas place one space, the entry written first wins.
- Spaces already on a shard with no map entry (written before sharding
  was on) are recorded where they are at startup.
- The change feed, leases, jobs, rate counts and live-editing logs stay in
  the main database. Each shard keeps its own page change log, with one
  search follower per shard.
- Without shards the same code runs over the one database, with every
  space at home and no map.

### `D175`: moving a space is an offline admin command

- `vellum admin move-space <id> --to <shard>` copies every row of the
  space into the new shard with Postgres `COPY`, in one transaction that
  first clears what an interrupted move left, records the move in the
  map, then deletes the rows from the old shard.
- Stars, recents, grants, versions and activity move with the space.
- Every replica is stopped first, since running replicas keep the old
  placement in memory.

## Alternatives

- **Redis counters**: exact and fast, but one more service to run for
  the HA profiles, when the index database already holds shared state.
- **A database write per request**: exact, but puts a round trip on
  every request and a hot row per busy caller.
- **Dividing the allowance by the number of replicas**: needs no shared
  state, but is wrong whenever traffic is uneven, which it usually is.
- **Hashing the space ID to pick a shard**: needs no map, but adding a
  shard would move most spaces, and a space could not be moved alone.
- **Moving spaces online**: needs every replica told of the move and
  writes paused for that space; not worth it before a real need.

## Consequences

- A limit of 100 a minute holds at about 100 across three replicas;
  the overshoot is what the cluster serves in the second before a count
  is shared.
- `rate_counts` holds callers' user IDs and addresses for two minutes,
  inside the index database that is already encrypted at rest on SQLite.
- A stolen cluster key would let its holder skip rate limits on the
  collab path; the key already lets its holder join any room, so it stays
  a secret of the same weight.
- Cross-space lists cost one query per shard. Starred items are merged
  per shard, so across shards they are no longer strictly newest first.
- A page moved into a space on another shard loses its stars, recents and
  outgoing links (rewritten on its next save); its activity stays on the
  old shard as history.
- MCP keeps its per-token limiter and in-process change events until the
  MCP server is mounted in serve; it will count through the same table.
