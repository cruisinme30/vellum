---
id: 01M63CJSGXEV1VK8E5593H182C
title: "ADR 0015: Collections engine and service"
status: verified
owner: "@cruisinme30"
tags: [adr, collections, engine, service]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D73, D74, D75, D132, D133, D134
- Issue: #125, #126, #127; views, CSV and embeds in #128 to #132
- Pull requests: #208 (engine), #266 (service)

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

Most of `vellum-collections` is pure: it parses and writes text and
computes values, with no I/O. Its `service` module serves that over the
gateway, reading and writing files through the blob store and checking
each request through an access port backed by authorization.

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

### D132: the service loads from the store and rewrites rows in place

- On first use the service reads every `*.collection/schema.yaml` and its
  `rows/*.md` into one workspace; the watcher calls `invalidate` when a
  file changes on disk, and the next request reloads.
- Each row and `schema.yaml` keep the etag they were read at. A save
  (a row, or a saved view in the schema) rewrites the file only if the
  etag still matches, so an outside edit is never lost; a mismatch is a
  `conflict` and drops the cache.
- New rows get `rows/<slug>.md`, then `-2`, `-3`… Deleted rows move to
  `.vellum/trash/collections/<collection>/<row>.md`.
- When two folders share a collection ID, the older `schema.yaml` keeps
  it and the copy is reported, not loaded.
- A CSV import is all or nothing: rows are applied to a copy before any
  file is written. A dry run reports the same result without writing.

### D133: reading or editing a collection covers what it links to

- Reading needs read access to the collection and to every collection
  its relations point at, since relation cells show linked titles and
  rollups show linked values. A collection the caller cannot read is
  `not_found`.
- Editing also needs edit access to the other side of each two-way
  relation, because the engine mirrors links into those row files.
- Saving or deleting a view changes only `schema.yaml`, so it needs edit
  access to that collection alone.
- Writes need the `write:direct` scope; reads need `read`. Creating
  a collection from CSV needs edit access to the drive folder.

### D134: `vellum-collections` is a service-layer crate

- The crate map already lists it as a service. The engine modules are
  its domain; `service` holds the ports (`BlobStore`, `Access`) and the
  operations, so the domain still has no I/O.

## Alternatives

- **Write computed values to front matter**: files grow, and merges and
  outside edits can leave them disagreeing with their inputs.
- **Store only one side of a relation**: a row file would not read on
  its own, and the index could not be rebuilt from either side.
- **Redact linked titles a caller cannot read**: safer for sharing, but
  every view and rollup would need per-cell checks; revisit with sharing.
- **A separate `vellum-collections-service` crate**: one more crate for
  no boundary gain, and the crate map names this one.

## Consequences

- The web app calls `collections.list`, `get`, `query`, `get_row` and
  the row writes, with JSON Schema for every input and output.
- Pages show `collection` blocks and `source=collection:` charts live
  through `collections.embed` and `collections.chart`, checked as the
  reader. A bad block is a readable `invalid_input` the page shows in
  place; an unreadable collection is `not_found`, never shown.
- Still to come: pushing computed rows to the index and search (D73),
  saving views, CSV and embed operations (#128 to #132).
