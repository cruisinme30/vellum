---
id: 01M64V9JQ1JJ0SY03AB2Z4YKTA
title: "ADR 0050: Work views, triage and cycles in the web app"
status: draft
owner: "@cruisinme30"
tags: [adr, work, web]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D220, D221, D222, D297
- Issue: #504, #524
- Pull requests: none yet

:::

## Context

The Work engine and API ([ADR 0016](0016-work-issue-tracking-core.md))
already group, order and filter issues, run triage, keep SLA clocks and
roll cycles with burn-up points. The web app showed only a team's open
issues in a flat list. R-WK-3, 4, 5, 6, 8 and 9 need the board, the
AND/OR filters, saved and shared views, My Issues in focus order, the
keyboard model, the triage inbox with SLA colours, and cycles with burn-up
and velocity. `vellum_work::view::View` existed, but nothing stored it, so
saved views had no server side.

## Decisions

### `D220`: saved views are Work files under `views/`

- A saved view is `views/<id>.yaml` in the Work store, the engine's `View`
  type as written, so it replicates through the Work journal like issues.
- `GET /v1/work/views`, `POST /v1/work/views`, `PUT` and `DELETE
  /v1/work/views/{view_id}`. A view is private, team or workspace
  visible; only its owner changes or deletes it.
- A view the caller cannot see answers "not found", so private view IDs do
  not leak. The filter is checked on save, as `query_issues` checks it.

### `D221`: one Issues screen, its settings in the address

- `/issues` shows a list or a board; team, layout, grouping, order,
  closed issues, the filter (JSON, as the API takes it) and the saved view
  all live in the query string, so any view is a link.
- The filter builder edits the API's tree: a group joined by "all" or
  "any", with one level of nested groups. A filter it cannot show (such
  as `not`) is kept as is and can be cleared.
- Cards move between status or priority columns by dragging or with
  Shift+arrow, announced to screen readers. J/K (and H/L on a board) move
  the focus; `C` and the `G` chords come from the shared shortcut table, which a
  screen now joins with `useScreenCommands`.
- While a change reloads the view, the last result stays on screen.

### `D222`: triage, cycles and My Issues are tabs beside Issues

- `/issues/mine`, `/issues/triage` and `/issues/cycles` share the tabs and
  the team picker. The triage inbox offers Accept (into a chosen status),
  Decline with a reason, Duplicate of a key, and Snooze for a day, three
  days or a week.
- SLA chips use the engine's risk colours and spell the risk out, so
  colour is never the only signal.
- The burn-up is a small SVG chart with a table of the same points;
  velocity is the engine's mean of the last three finished cycles.

### `D297`: team settings edit the whole team file

- `/issues/settings` edits statuses, cycles, triage rules, SLAs,
  estimates and the Git mapping, reading `GET
  /v1/work/teams/{team}/settings` and saving the whole file with `PUT
  /v1/work/teams/{team}`, which validates it first.
- Removing a status with issues asks where they go; the moves run on
  save, before the status disappears. A save never lowers `last_number`
  and never changes the team ID.
- The Git mapping lives in `team.yaml` and wins over
  `.vellum/integrations.yaml`, read on every event, so edits apply live.

## Alternatives

- **Saved views in browser storage**: rejected; R-WK-4 needs shared views,
  and a view must follow its owner across devices.
- **A text query language in the filter bar**: rejected by D15; the
  builder edits the same tree the server stores.
- **A charting library for the burn-up**: rejected; one line chart does
  not justify the bundle size.

## Consequences

- Team settings stay admin-only with fresh sign-in; team leads cannot
  save them yet. Archiving a team has no server support yet.
- A board grouped by assignee, label or project does not move cards; those
  changes stay on the issue page.
- Projects, insights and intake screens are a separate workstream and
  reuse the chips and the tabs.
