---
id: 01M63CFE68RDVE0FK3FPM39SCP
title: Plans, timelines and risk
vellum: 1
---

# Plans, timelines and risk

## Timeline

Each line is `date | title | detail`; the detail is optional.

```timeline
Oct 6 | Design approved | Scope: 7-day replay
Oct 20 | Replay in staging
Nov 24 | GA | Blocked on R3
```

`````markdown {corpus="fence/timeline"}
```timeline
Oct 6 | Design approved | Scope: 7-day replay
Oct 20 | Replay in staging
Nov 24 | GA | Blocked on R3
```
`````

## Roadmap

A Gantt-style roadmap with a line for today. `range:` sets the visible
window; each line is `name | start | end`, and `milestone` in place of an
end date draws a diamond.

```roadmap
range: 2026-11-01 .. 2027-06-30
Discovery | 2026-11-01 | 2026-12-15
Build | 2026-12-01 | 2027-04-30
Beta | 2027-03-01 | 2027-05-10
Launch | 2027-05-15 | milestone
```

`````markdown {corpus="fence/roadmap"}
```roadmap
range: 2026-11-01 .. 2027-06-30
Discovery | 2026-11-01 | 2026-12-15
Build | 2026-12-01 | 2027-04-30
Beta | 2027-03-01 | 2027-05-10
Launch | 2027-05-15 | milestone
```
`````

## Risk matrix

Each line is `risk | likelihood n | impact n`, with both scores from 1
to 5. Risks land in the matching cell of a five-by-five grid.

```risk-matrix
PDF export slips | likelihood 3 | impact 4
Drive quota runs out | likelihood 2 | impact 2
Key person leaves | likelihood 1 | impact 5
```

`````markdown {corpus="fence/risk-matrix"}
```risk-matrix
PDF export slips | likelihood 3 | impact 4
Drive quota runs out | likelihood 2 | impact 2
Key person leaves | likelihood 1 | impact 5
```
`````

## Board

A static board inside a page: `##` starts a column, each bullet is a
card, and a tag in brackets becomes a chip. For a board you can drag
cards on, use a [collection](../data/views.md) or an
[issue board](../work/README.md).

```board
## To do
- Write the migration guide [P1]
## Doing
- Replay engine [P0]
## Done
- Diff report
```

`````markdown {corpus="fence/board"}
```board
## To do
- Write the migration guide [P1]
## Doing
- Replay engine [P0]
## Done
- Diff report
```
`````
