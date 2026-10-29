---
id: 01M63CFE5ZAB00T8DNFJ9PV6M4
title: Choosing a deployment
vellum: 1
---

# Choosing a deployment

Vellum climbs a ladder. Start on the lowest rung that fits and move up
when you need to; every rung reads the same files, so moving up is a
migration of the index, not of your content.

| Profile | Runs as | Index | Content | Fits |
| --- | --- | --- | --- | --- |
| single | One process | SQLite (encrypted) | A local folder | One person to a small team |
| single | `docker run` | SQLite (encrypted) | A mounted volume | A small team on one server |
| team | `docker compose` | Postgres | S3-compatible (MinIO) | Up to a few hundred people |
| org | Helm on Kubernetes | Postgres with a replica | S3-compatible | An organisation, 99.9% availability |

```mermaid
flowchart LR
  S[Single binary] --> D[docker run]
  D --> C[docker compose]
  C --> H[Helm]
```

## What each profile promises

::: facts

- single: under 80 MB of memory idle, cold start under 2 seconds, no other services
- team: 50 people editing at once and 2,000 page reads per second on one host
- org: 99.9% monthly availability, no single point of failure, database recovery point of zero, recovery time under 15 minutes

:::

## Moving up a rung {#migrate}

1. Take a backup with `vellum backup`.
2. Start the new profile against the same content, or copy the content to its new storage with `vellum migrate-storage`.
3. Run `vellum reindex` to build the index in the new database.
4. Point your DNS at the new deployment.

The index is always rebuildable from the files, so it never needs to be
migrated row by row.
