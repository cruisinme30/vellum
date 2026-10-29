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
- Issue: `#152` (with `#153`–`#156`, `#158`, `#159`, `#161`, `#163`)
- Pull requests: `#243`

:::

## Context

My Work (brief §9, R-MW-1 to R-MW-8) is a personal task view built from
the checklists people already write, unioned with assigned issues,
comments and review requests. Files must stay readable and editable by
hand, so task fields have to live in the Markdown itself. The crate owns
no storage: it reaches files, people and secrets through a `Workspace`
port.

## Decisions

### `D82`: task fields live in a trailing attribute span

- A task is a checklist item with a `^blk-` ID; its fields are a `{…}`
  span before the ID, keys in the order `when start due rrule recur_from
  remind p labels list done src`. Unknown keys are kept.
- An inline ISO date is the deadline when there is no `due` (D19);
  `@handle` mentions are assignees; nested items are subtasks.
- Completing a repeating task moves it to its next date and leaves it
  open; dragging to a bucket moves the start date and keeps the deadline.
- Every change is a text edit, written back with the version it was read
  at and retried on conflict.
- Why: the file stays the single source of truth and survives any editor.

### `D83`: a VEVENT feed and VTODO CalDAV

- The secret feed `/feeds/tasks/<token>.ics` emits all-day DATE events and
  30-minute transparent UTC events, looking back 30 days.
- Only schedule-based repeats carry an RRULE.
- The token is a share token; only its hash is stored, and rotating it
  revokes the old one.
- CalDAV (`/dav/…`) serves each list as `VTODO` items in floating local time;
  sync tokens are list versions, and a stale token gets `403
  valid-sync-token`. Client UIDs are kept as `src=caldav:<uid>`.
- Why: this is what calendar apps and task clients actually read, with no
  extra storage for change logs.

### `D84`: imports and quick-add

- Imports read Google Takeout `Tasks.json`, the Todoist CSV and Things
  JSON; each item records `src`, so a re-run adds only new items.
- Quick-add is English only for 1.0; a weekday means the next one after
  today, and bare abbreviations count only after a prefix.
- Slash dates are refused until locale packs; quoted text is never parsed.
- Property tests guard these rules.
- Why: predictable parsing in a box people type into many times a day.

## Alternatives

- **A sidecar index for fields**: breaks "files are the truth". Rejected.
- **Dataview-style inline fields**: collide with prose. Rejected.
- **`VTODO` items in the subscribed feed**: most calendar apps ignore them.
- **Tombstones for CalDAV sync**: need storage the crate does not own.

## Consequences

- Editors, scripts, CalDAV clients and imports all make the same edits.
- The server binary implements `Workspace` and mounts the raw routes.
- Time-blocking (`#162`) and non-English quick-add are follow-ups.
