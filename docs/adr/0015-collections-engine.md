---
id: 01M63CJSGXEV1VK8E5593H182C
title: "ADR 0015: Collections engine"
status: verified
owner: "@cruisinme30"
tags: [adr, collections, engine]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D73, D74, D75
- Issue: #125, #126, #127; views, CSV and embeds in #128 to #132
- Pull requests: #208

:::

## Context

The brief (section 5) fixes what a collection is: a folder with
`schema.yaml` and one page per row in `rows/*.md`, thirteen property
types, two-way relations, nine rollup aggregations, a safe formula
language computed on the server when a row is saved, six view layouts,
CSV import and export, the `collection` fence and live chart sources. It
leaves open where computed values live, how both sides of a relation stay
in step when files change, and the exact formula and filter semantics.
The web, render, search and MCP threads build against this crate in
parallel, so these have to be settled now.

The crate `vellum-collections` is pure: it parses and writes text and
computes values, with no I/O. The collections service reads and writes
files through the blob store, checks permissions through authorization,
and pushes computed rows to the index and search.

## Decisions

### D73: computed values are never written to row files

- Formula, rollup and created values are derived on every save and kept
  in the index, never in front matter. Created comes from the row ID's
  time (D37), so it needs no field either.
- Row files carry only what people typed. External edits, Git merges and
  sync conflicts never touch derived data, so they cannot disagree with it.
- The save path is: apply an edit, write the changed row files, then
  recompute the affected rows: the changed rows plus, transitively, every
  row that links to them from a collection with rollups.
- Schema files store property keys (short, stable) in views, filters and
  sorts; display names may change without rewriting anything else.
  Filters and sorts are written as text (`st != Done`, `e desc`) and accept
  either keys or names when read.

| Option | Files stay small | Merge-safe | Readable on GitHub | Chosen |
| --- | --- | --- | --- | --- |
| Derive on save, keep in the index | ✓ | ✓ | ◐ | ✓ |
| Write computed values to front matter | ✗ | ✗ | ✓ | ✗ |

### D74: both sides of a two-way relation are stored and kept in step

- A relation names its target collection (`to`) and, when two-way, the
  paired relation property there (`back`). The pair is checked when both
  collections are loaded.
- Both row files list their links, so each file reads on its own and the
  index can be rebuilt from either side.
- Every change goes through the engine, which mirrors the difference to
  the other side: an edit, a new row, or a file changed on disk (compared
  with its previous version). Deleting a row removes every link to it,
  one-way links included. Links to rows that do not exist are dropped
  when a changed file is read.

### D75: formula, rollup and filter semantics

- Formulas cannot loop, call out, or read anything but the current row.
  Limits: 4,096 characters, nesting depth 64, 100,000 evaluation steps,
  64 KiB of text. Every error carries a character span so the cell and
  the formula editor show it inline.
- Arithmetic treats an empty value as 0 and `+` joins text. Dates move by
  whole days (`date + 7`), and `date - date` and `days(a, b)` give whole
  days from `b` to `a`. Comparisons with an empty value are false.
  Division by zero, mismatched types and out-of-range numbers are cell
  errors. An unset checkbox is unchecked.
- Bare names must be one word; `prop("Due date")` reaches any property.
  Names resolve to keys when the schema loads, so renaming a property
  never breaks a formula's meaning, and loops between formulas are
  rejected then. Loops that cross collections through rollups surface as
  a "circular reference" cell error.
- Rollups: `count` counts related rows; `sum avg min max` use numbers
  (`min max` also dates); `checked` and `pct` count checked values (`pct`
  as a fraction, shown as a percentage); `unique` counts distinct values;
  `show` lists them.
- Filters: `is`, `not`, `contains`, `not contains`, `is empty`,
  `is not empty`, `< <= > >=` (dates accept `@today`). Text matching
  ignores case; lists match when any item matches. Sorts put empty values
  last and order select options as the schema lists them.
- CSV export prefixes text a spreadsheet would run as a formula with `'`.

## Consequences

- The web thread gets typed results (`ViewResult`, `ComputedRow`,
  `ChartData`) with JSON Schema, ready for the service layer's API.
- The render thread calls `CollectionEmbed` and `ChartSource` to resolve
  `collection` fences and `source=collection:` charts, and shows an
  `EmbedError` as a readable fallback.
- The collections service is the next step: it loads folders through the
  blob store, applies edits, and exposes them as service operations.
