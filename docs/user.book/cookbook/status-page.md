---
id: 01M63CFE6FK1VH1H0A6T3AHZYM
title: A weekly status page
vellum: 1
---

# A weekly status page

**You will build** a status page that leadership can read in thirty
seconds: headline numbers, one chart, what changed and what is at risk.

## Steps

1. Create a page titled "Gateway status, week 45".
2. Type `/stats` and add three tiles: a value, a label and a change since last week.
3. Type `/chart`, choose **Line** and paste the week's p95 latency.
4. Add a `warn` callout for anything at risk and a `good` callout for wins.
5. Next week, open **⋯ → Duplicate** and update the numbers.

## The finished page

::: stats

- 99.97% · Availability · ▲ 0.02 pts
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

`````markdown {corpus="page/cookbook-status-page" live=false}
---
id: 01JB9C2D4E6F8G0H2J4K6M8N0P
title: Gateway status, week 45
owner: sam
tags: [status, gateway]
vellum: 1
---

::: stats

- 99.97% · Availability · ▲ 0.02 pts
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
`````
