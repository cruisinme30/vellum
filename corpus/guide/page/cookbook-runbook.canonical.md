---
id: 01JB9E4F6G8H0J2K4M6N8P0Q2R
title: "Runbook: gateway latency"
icon: 🚨
status: verified
owner: sam
tags:
- runbook
- on-call
verify_every: 90d
vellum: 1
---

::: crit

Never roll back the audit writer separately from the gateway.

:::

## Decide

```mermaid
flowchart TD
  A[p95 over 150 ms for 10 min] --> B{Deploy in the last hour?}
  B -->|yes| C[Roll back the deploy]
  B -->|no| D{CPU over 80%?}
  D -->|yes| E[Scale out]
  D -->|no| F[Page the gateway team]
```

## Size the scale-out

```calculator
rps: Current requests per second = 4000 [500..20000 step 500]
per: Requests per replica = 900 [200..2000 step 50]
result: Replicas needed = round(rps / per + 1) (replicas)
```

## Roll back

::: steps

1. **Find the last good release** with `helm history gateway -n prod`
2. **Roll back** with `helm rollback gateway <revision> -n prod`
3. **Watch** `kubectl rollout status deploy/gateway -n prod`
4. **Announce** in the incidents channel and open an OPS issue

:::
