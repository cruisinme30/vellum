---
id: 01M63CFE62RE8A1SRHR8WKGA4T
title: Service levels
vellum: 1
---

# Service levels

These are the targets Vellum is built and tested against. The nightly
performance suite fails the build when a p95 regresses by more than 10%.

## Latency {#latency}

| Operation | p95 | p99 |
| --- | --- | --- |
| Cached page read | under 50 ms | under 120 ms |
| Uncached page read | under 150 ms | |
| Keystroke reaching other editors | under 200 ms | |
| Render a 50-block page | under 300 ms | |
| Export a 30-page PDF | under 3 s | |

## Availability and recovery {#availability}

::: facts

- Availability: 99.9% a month on the org profile, with no single point of
  failure
- Database recovery point: zero (synchronous replica)
- Content recovery point: 15 minutes or less
- Recovery time: under 15 minutes
- Collaboration room failover: under 5 seconds
- Permission changes visible: under 1 second

:::

## Footprint {#footprint}

::: facts

- Single binary idle: under 80 MB of memory and under 1% CPU
- With 10 open collaboration rooms: under 250 MB
- Download: under 40 MB
- Cold start: under 2 seconds
- Team profile: under 2 GB of memory in total

:::

## When parts fail {#degradation}

| Failure | What users see |
| --- | --- |
| Object storage down | Pages still open from the rendered cache; saves queue; a banner explains |
| Kroki down | Last drawing plus source for Kroki diagrams |
| Redis down | Nothing; caches fall back to memory |
| Identity provider down | Signed-in people carry on; break-glass admins can sign in |
| A replica dies | Requests move to the others; collaboration rooms re-home in under 5 s |
