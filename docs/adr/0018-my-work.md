---
id: 01M63DRA9X5B7BQYA5GKM9ESZE
title: "ADR 0018: My Work tasks, calendars, imports and capture"
status: draft
owner: "@cruisinme30"
tags: [adr, my-work, tasks]
vellum: 1
---

::: facts

- Date: `2026-10-29`
- Status: accepted
- Decisions: `D82`, `D83`, `D84`
- Issue: `#152`, with `#153`, `#154`, `#155`, `#156`, `#158`, `#159`,
  `#161` and `#163`
- Crate: `vellum-tasks` (layer `service`)

:::

## Context

My Work (brief §9, R-MW-1 to R-MW-8) is a personal task view built from
the checklists people already write. The brief makes the Markdown
checklist block the source of truth, linked by its block ID, and unions
it with assigned issues, comment assignments and review requests. On top
of that come buckets, start and due dates, repeats, subtasks, reminders
and a digest, a natural-language quick-add, a calendar feed and CalDAV,
one-way imports, shared lists and capture from e-mail and Slack.

Two constraints shaped every choice. Files stay readable and editable by
hand, so task fields must live in the Markdown and survive any editor.
And the crate owns no storage: it reaches files, people and secrets only
through a `Workspace` port, so it can be built and tested before the
store, index and identity services are wired together.

## Options considered

- **Fields in a sidecar index**: keeps the Markdown clean but breaks the
  "files are the truth" rule and loses fields on a plain copy. Rejected.
- **Fields as inline emoji or Dataview-style `[due:: …]`**: readable, but
  collides with ordinary prose and needs a bespoke grammar. Rejected for a
  Pandoc-style attribute span, which the dialect already parses.
- **A VTODO feed for calendars**: few calendar apps show VTODOs in a
  subscribed feed, so the feed uses VEVENTs and CalDAV carries VTODOs.
- **Change logs for CalDAV sync**: tombstones would need storage the crate
  does not own. Sync tokens are list versions instead, and a stale token
  asks the client to resync.

## Decisions

### `D82`: task fields live in a trailing attribute span

- A task is a checklist item with a block ID. Its fields are a trailing
  `{…}` span before `^blk-…`, with keys in the order `when`, `start`,
  `due`, `rrule`, `recur_from`, `remind`, `p`, `labels`, `list`, `done`,
  `src`. Unknown keys are kept as written.
- An inline ISO date in the title is the deadline when there is no `due`
  (D19). `@handle` mentions are assignees; nested items are subtasks.
- Completing a repeating task moves it to its next date in place and
  leaves it open. Dragging to a bucket moves the start date and keeps the
  deadline.
- Why: the file stays the single source of truth, and the span is
  ordinary Vellum Markdown that survives round trips.

### `D83`: calendars

- The secret feed (`/feeds/tasks/<token>.ics`) emits VEVENTs: all-day
  tasks as DATE events, timed tasks as 30-minute transparent UTC events.
  Only schedule-based repeats carry an RRULE; it looks back 30 days. The
  token is a share token whose hash is stored; rotating revokes the old.
- CalDAV (`/dav/…`) serves each list as a VTODO collection with floating
  local times. Sync tokens are list versions; a stale token gets `403
  valid-sync-token`. Tasks a client creates keep its UID as
  `src=caldav:<uid>`.
- Why: this is what Google, Apple and Outlook calendars and DAVx5/Tasks
  actually read, with no extra storage.

### `D84`: imports and quick-add

- Imports read Google Takeout `Tasks.json`, the Todoist CSV template and
  Things JSON (`things:///json`). Each item records `src`, so running an
  import again adds only new items.
- Quick-add is English only for 1.0. Bare weekday abbreviations count
  only after a prefix such as `on` or `by`; a weekday always means the
  next one after today; slash dates are refused until locale packs; text
  in double quotes is never parsed. Property tests guard these rules.
- Why: predictable parsing beats clever parsing in a box people type into
  fifty times a day.

## Consequences

- Any editor, script or Git user can read and change tasks; the web app,
  CalDAV clients and imports all make the same text edits, written back
  with optimistic versioning and retried on conflict.
- The `Workspace` port is the integration point: the binary implements it
  over the store, index, identity and authorization, and mounts the raw
  routes on the gateway. That wiring lands with the server binary.
- Time-blocking with Google Calendar (`#162`) and non-English quick-add
  are follow-ups.

## History

- `2026-10-29`: proposed with the My Work pull request.
