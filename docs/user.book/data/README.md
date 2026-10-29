---
id: 01M63CFE6AY4JH4VRWJD27AB60
title: Collections
vellum: 1
---

# Collections

A **collection** is a database whose rows are pages. Use one wherever
you would reach for a spreadsheet of records: requirements, vendors,
candidates, decisions, releases. Each row has properties you can sort,
filter and group, and a body for notes, comments and attachments.

## A collection is a folder

On disk a collection is a folder ending in `.collection`, holding one
`schema.yaml` and one Markdown file per row:

```text
requirements.collection/
  schema.yaml
  rows/R1-replay-7-days.md
  rows/R2-diff-report.md
  rows/R3-pdf-report.md
```

Because every row is a file, a collection survives without Vellum, diffs
cleanly in Git and takes the same permissions as any other folder.

## Create a collection {#create}

1. Select **New → Collection**, or type `/collection` in a page to create one inline.
2. Name it, for example "Requirements tracker".
3. Add properties with **+** at the end of the header row.
4. Add rows with **New row**, or paste rows from a spreadsheet.

## The schema

The schema lists the properties and the saved views. The editor writes
it for you; this is what a requirements tracker's schema looks like.

```yaml
title: Requirements tracker
properties:
  - {key: t, name: Requirement, type: title}
  - {key: pr, name: Priority, type: select, options: [P0, P1, P2]}
  - {key: st, name: Status, type: select, options: [Open, In review, Done, Blocked]}
  - {key: own, name: Owner, type: person}
  - {key: e, name: Target, type: date}
  - {key: est, name: Estimate, type: number}
  - {key: pg, name: Progress, type: number, format: percent}
  - {key: rel, name: Release, type: relation, to: 01JB5C2V8N4R6T0W3Y5Z7Q9K1M}
  - {key: f1, name: Remaining, type: formula, f: 'round(Estimate * (1 - Progress), 1)'}
views:
  - {name: All, type: table}
  - {name: By status, type: board, group: st}
```

## A row

Each row is a page whose front matter holds the property values, keyed
by the property `key`. Formula and rollup values are computed, never
stored.

````markdown {corpus="page/collection-row" live=false}
---
id: 01JB5D0F3H6K9M2P4R7T1V3X5Z
t: R1 · Replay 7 days
pr: P0
st: Done
own: dev
e: 2026-10-28
est: 8
pg: 1
rel: [01JB5C2V8N4R6T0W3Y5Z7Q9K1M]
vellum: 1
---

Notes about this requirement.
````

## Import and export CSV {#csv}

**⋯ → Import CSV** creates rows from a CSV file and maps columns to
properties; Vellum suggests a type for each new column. **⋯ → Export
CSV** writes the current view, with its filters and sort, to a file.

## In this part

- [Properties and relations](properties.md): every property type, relations and rollups.
- [Formulas](formulas.md): the formula language with 30 worked examples.
- [Views, filters and embeds](views.md): table, board, gallery, calendar, timeline and list.
