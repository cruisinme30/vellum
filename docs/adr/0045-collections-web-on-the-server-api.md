---
id: 01KB46M7KGNNWYWK1N9V4P1EKA
title: "ADR 0045: Collections web screens on the server API"
status: verified
owner: "@cruisinme30"
tags: [adr, collections, web]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D190, D191, D192, D193
- Issue: #474
- Pull requests: to be linked when merged

:::

## Context

The web collection views (ADR 0010, PR #378) ran on the server for reads and
row edits only. The server already served row deletes, saved views, CSV and
embeds (ADR 0015), but the web port did not expose them, so a person could
not delete a row, change a view, move data in or out, or make a collection
without editing files. Nothing on the server could change a collection's
columns, and edits made in another tab only showed after a reload. The brief
asks for filters, sorts, grouping, hidden columns, CSV import and export,
and live `collection` blocks in pages (section 5).

## Decisions

### `D190`: properties change through three schema operations

- `collections.add_property` (POST `/v1/collections/{id}/properties`),
  `update_property` (PATCH `…/properties/{key}`) and `delete_property`
  (DELETE `…/properties/{key}`), all `write:direct` and edit access only.
- They reuse the etag-guarded `schema.yaml` rewrite that saved views use, and
  the changed schema is re-parsed as if loaded from disk, so a bad formula or
  view fails the same way a hand edit would.
- The server picks a new property's key from its name; keys and types never
  change afterwards, so row files stay valid.
- Relations (two-way, they touch two schemas) and a second title are refused;
  a rename is refused while a formula names the property, because formulas
  store names in their source.
- A delete drops the view filters, sorts, grouping and dates that used the
  property and is refused while a formula or rollup reads it. Row files keep
  the old field until the row is next saved.

### `D191`: views stay fresh without a push channel

- The HTTP adapter tells open views to reload after this tab's writes, when
  the tab becomes visible or focused, and every 30 seconds while visible.
- Reloads closer than two seconds apart are skipped, and hidden tabs never
  poll, so an idle tab costs nothing.
- A server-sent change feed can replace the timer later without touching
  views: they only see `subscribe`.

### `D192`: the web port mirrors every collections operation

- `CollectionsStore` gains `deleteRow`, `saveView`, `deleteView`, the three
  property calls, `exportCsv`, `importCsv`, `createFromCsv` and `embed`.
- The mock adapter implements all of them on the Northstar demo, so every
  screen works without a server and in unit tests.
- A type test compares the hand-written types with the generated client
  (`api.gen.ts`) in both directions, so server changes break the build
  rather than the screen.
- Screens: an Edit view / New view side panel, a Properties panel, CSV import
  with a dry-run check, CSV export of the open view, row delete, and New
  collection (blank, or inferred from a CSV file) on the Data page.

### `D193`: collection blocks preview live rows while editing

- The editor's `collection` fence calls `collections.embed` as the reader, so
  the preview obeys the same permissions as the rendered page.
- It asks again 400 ms after typing stops and shows at most eight rows; the
  saved page still renders on the server.

## Alternatives

- **Generic `update_schema` taking YAML**: one route, but it hands clients the
  file format and makes partial edits race; typed operations validate intent.
- **Rewriting formulas on rename**: friendlier, but a text rewrite of
  expressions risks silent changes; refusing with the formula's name is safe.
- **WebSocket or SSE change feed now**: the collab socket is per document and
  the cluster feed is server-side; polling a visible tab is enough for 1.0.
- **Generated types only**: the generated schema is looser (strings for
  enums, optional for `Option`), which would weaken every view's types.

## Consequences

- People manage columns, views and CSV from the browser; `schema.yaml` stays
  the single source of truth.
- Changing a property's type, adding relations and editing rollups still
  need `schema.yaml`; follow-up work if users ask.
- Visible tabs reload open collection views every 30 seconds; with very
  large views this is a load to watch in the k6 runs.
