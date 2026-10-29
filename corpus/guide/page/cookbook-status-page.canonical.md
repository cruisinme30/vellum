---
id: 01JB9C2D4E6F8G0H2J4K6M8N0P
title: Gateway status, week 45
owner: sam
tags:
- status
- gateway
vellum: 1
---

::: stats

- 99\.97% · Availability · ▲ 0.02 pts
- 38 ms · p95 latency · ▼ 4 ms
- 3 · Incidents · ▬ 0

:::

```chart
title: p95 latency this week
type: line
unit: ms
labels: [Mon, Tue, Wed, Thu, Fri]
values: [44, 41, 40, 39, 38]
```

::: good

The new cache cut read latency by 9%.

:::

::: warn

The EU region migration slips a week while we wait for the new database
quota (OPS-31).

:::
