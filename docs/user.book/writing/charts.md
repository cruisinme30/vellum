---
id: 01M63CFE68K4DQC96RHGRDB3BH
title: Charts and data tables
vellum: 1
---

# Charts and data tables

A `chart` fence draws a chart from a few lines of YAML. Charts follow
the page theme, read aloud as a data table in screen readers and export
as vector images to PDF and Word.

## Options

::: facts

- title: the heading above the chart
- type: bar, line, area, donut, pie, grouped, stacked or scatter
- labels: the x-axis categories, for bar, line, area, grouped and stacked
- values: one series of numbers, for bar, line and area
- any other key: a named series (grouped, stacked, scatter) or slice (donut,
  pie)
- unit: a suffix for values, such as ms or %
- ymin and ymax: fix the y-axis range
- legend: top, bottom, right or none
- colors: a list of theme colour names or hex values

:::

## Bar

```chart
title: Blocked requests per week
type: bar
labels: [W40, W41, W42, W43]
values: [120, 185, 160, 240]
```

`````markdown {corpus="fence/chart-bar"}
```chart
title: Blocked requests per week
type: bar
labels: [W40, W41, W42, W43]
values: [120, 185, 160, 240]
```
`````

## Line

```chart
title: p95 latency
type: line
unit: ms
labels: [Mon, Tue, Wed, Thu, Fri]
values: [52, 48, 50, 41, 38]
```

`````markdown {corpus="fence/chart-line"}
```chart
title: p95 latency
type: line
unit: ms
labels: [Mon, Tue, Wed, Thu, Fri]
values: [52, 48, 50, 41, 38]
```
`````

## Area

```chart
title: Requests served (millions)
type: area
labels: [Jul, Aug, Sep, Oct]
values: [3.1, 3.4, 4.0, 4.6]
ymin: 0
```

`````markdown {corpus="fence/chart-area"}
```chart
title: Requests served (millions)
type: area
labels: [Jul, Aug, Sep, Oct]
values: [3.1, 3.4, 4.0, 4.6]
ymin: 0
```
`````

## Grouped

Each extra key is a series, drawn side by side.

```chart
title: Before and after the cache
type: grouped
labels: [Read, Write, Search]
Before: [48, 120, 95]
After: [21, 110, 60]
```

`````markdown {corpus="fence/chart-grouped"}
```chart
title: Before and after the cache
type: grouped
labels: [Read, Write, Search]
Before: [48, 120, 95]
After: [21, 110, 60]
```
`````

## Stacked

```chart
title: Issues closed per cycle
type: stacked
labels: [C01, C02, C03, C04]
Bugs: [8, 6, 9, 4]
Features: [5, 7, 6, 9]
Chores: [2, 3, 1, 2]
legend: bottom
```

`````markdown {corpus="fence/chart-stacked"}
```chart
title: Issues closed per cycle
type: stacked
labels: [C01, C02, C03, C04]
Bugs: [8, 6, 9, 4]
Features: [5, 7, 6, 9]
Chores: [2, 3, 1, 2]
legend: bottom
```
`````

## Donut

Each key is a slice.

```chart
title: Catch rate by attack class
type: donut
Direct: 60
Indirect: 30
Multi-turn: 10
```

`````markdown {corpus="fence/chart-donut"}
```chart
title: Catch rate by attack class
type: donut
Direct: 60
Indirect: 30
Multi-turn: 10
```
`````

## Pie

```chart
title: Traffic by region
type: pie
Europe: 45
Americas: 35
Asia Pacific: 20
unit: "%"
```

`````markdown {corpus="fence/chart-pie"}
```chart
title: Traffic by region
type: pie
Europe: 45
Americas: 35
Asia Pacific: 20
unit: "%"
```
`````

## Scatter

Each key is a series of `[x, y]` points.

```chart
title: Latency against payload size
type: scatter
x: Payload (KB)
y: Latency (ms)
Gateway: [[2, 21], [8, 30], [16, 38], [32, 61]]
Legacy proxy: [[2, 35], [8, 52], [16, 70], [32, 118]]
```

`````markdown {corpus="fence/chart-scatter"}
```chart
title: Latency against payload size
type: scatter
x: Payload (KB)
y: Latency (ms)
Gateway: [[2, 21], [8, 30], [16, 38], [32, 61]]
Legacy proxy: [[2, 35], [8, 52], [16, 70], [32, 118]]
```
`````

## Live charts from a collection

Point a chart at a [collection](../data/README.md) instead of typing the
numbers, and it redraws whenever a row changes. `x` and `y` name the
properties to plot; `agg` sums, counts or averages rows that share an
`x` value.

```chart {source=collection:01JB4Q8ZK2W6T9R3M5N7P1X0YC x=Target y=Estimate agg=sum}
title: Points due per week
type: bar
```

`````markdown {corpus="fence/chart-live"}
```chart {source=collection:01JB4Q8ZK2W6T9R3M5N7P1X0YC x=Target y=Estimate agg=sum}
title: Points due per week
type: bar
```
`````

## Data tables

A `table` fence holds CSV. Readers select a header to sort; `bar=`
draws an inline bar in a column, and `sum=` adds a total row.

```table sortable bar=Share sum=Value
Name, Value, Share
Alpha, 120, 40
Beta, 90, 30
Gamma, 90, 30
```

`````markdown {corpus="fence/table"}
```table sortable bar=Share sum=Value
Name, Value, Share
Alpha, 120, 40
Beta, 90, 30
Gamma, 90, 30
```
`````
