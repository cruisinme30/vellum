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
keys:
  kms: aws-kms:arn:aws:kms:eu-west-1:000000000000:key/northstar-vellum
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
| Redis (optional) | Cache only; Vellum falls back to memory |
| Kroki (optional) | Two replicas; pages fall back to the last drawing |
| ClamAV (optional) | Two replicas; uploads are refused while none is up ([virus scanning](compose.md#clamav)) |
