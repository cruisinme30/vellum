---
id: 01M63CFE60KC3WWME4NBEXXS0G
title: Kubernetes with Helm
vellum: 1
---

# Kubernetes with Helm

The **org** profile runs every stateless service with at least two
replicas, Postgres with a synchronous replica and automatic failover,
and shards collaboration rooms and search by space.

```bash
helm install vellum oci://ghcr.io/cruisinme30/charts/vellum \
  --namespace vellum --create-namespace \
  --values northstar-values.yaml
```

```yaml
# northstar-values.yaml
publicUrl: https://vellum.northstar.dev
profile: org
replicas:
  gateway: 2
  api: 3
  collab: 2
  render: 2
postgres:
  external: true
  url: postgres://vellum@pg-primary.db/vellum
  sslmode: verify-full
storage:
  url: s3://northstar-vellum-content
encryption:
  kms: aws-kms:eu-west-1:arn:aws:kms:eu-west-1:000000000000:key/northstar-vellum
serviceAccount:
  annotations:
    eks.amazonaws.com/role-arn: arn:aws:iam::000000000000:role/northstar-vellum
```

## Rollouts {#rollouts}

Upgrades roll out as a canary: 10% of traffic, then 50%, then 100%,
with automatic rollback if error rates or latency budgets are breached.
Pods drain gracefully, finishing in-flight requests and handing
collaboration rooms to another replica in under five seconds.

## What must be highly available {#ha}

| Component | How |
| --- | --- |
| Gateway, API, render, export | Two or more replicas behind the service |
| Collaboration rooms | Recreated from Postgres on another replica |
| Postgres | Primary plus synchronous replica, Patroni failover |
| Object storage | The provider's own redundancy |
| Redis | Not used: rate limits, jobs and the change feed live in Postgres |
| Kroki (optional) | Two replicas; pages fall back to the last drawing |
| ClamAV (optional) | Two replicas; uploads are refused while none is up ([virus scanning](compose.md#virus-scanning)) |

## When a dependency fails {#degraded}

`/readyz` reports each part as `ready`, `degraded` or `not_ready`. Only
`not_ready` takes a pod out of the load balancer.

| Failure | What people see | Readiness |
| --- | --- | --- |
| Object storage down | Search, lists, Work and My Work keep working; opening or saving a page fails with "storage is unavailable" and the editor retries | `degraded`: every replica shares the bucket, so none is taken out |
| Postgres primary lost | A few seconds of failed requests while Patroni promotes the replica | `not_ready` until the pool reconnects |
| Kroki down | Diagrams show their last drawing; after five failures Vellum stops asking for ten seconds | `ready` |
| ClamAV down | Uploads are refused; everything else works | `ready` |
| A pod stops | Readiness fails at once, the pod finishes its requests, saves and closes its live rooms; editors reconnect to another replica | `draining` |

The weekly chaos test cuts the servers off the bucket for twenty seconds,
along with killing a pod, a Kroki pod and the Postgres primary, and
checks that every acknowledged save survived.
