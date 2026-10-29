---
id: 01M63CFE69ZGB3H7PKY85FNHEE
title: Interactive blocks
vellum: 1
---

# Interactive blocks

## Calculator

A `calculator` fence turns a formula into sliders. Each input line is
`key: Label = default [min..max step n]`; the `result` line is a
[formula](../data/formulas.md) over the input keys, with an optional
unit in brackets. Readers drag the sliders; the page itself never
changes. Exports print the current values.

```calculator
units: Requests per second = 2000 [100..10000 step 100]
cost: Cost per million requests = 0.40 [0.10..2.00 step 0.05]
result: Monthly cost = units * 2.592 * cost ($)
```

`````markdown {corpus="fence/calculator"}
```calculator
units: Requests per second = 2000 [100..10000 step 100]
cost: Cost per million requests = 0.40 [0.10..2.00 step 0.05]
result: Monthly cost = units * 2.592 * cost ($)
```
`````

A month has about 2.592 million seconds, which is where the constant
comes from.

## Collection views

A `collection` fence embeds a live view of a
[collection](../data/README.md). Edits in the embedded view change the
rows themselves, and everyone sees them. `view` picks a saved view;
`filter` and `sort` narrow it further for this page only.

```collection
id: 01JB4Q8ZK2W6T9R3M5N7P1X0YC
view: By status
filter: Status != Done
sort: Target asc
```

`````markdown {corpus="fence/collection"}
```collection
id: 01JB4Q8ZK2W6T9R3M5N7P1X0YC
view: By status
filter: Status != Done
sort: Target asc
```
`````

Readers who cannot see the collection see a notice instead of the rows.
Exports show the rows as a table, as they were at export time.
