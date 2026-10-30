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
- Decisions: `D82`, `D83`, `D84`, `D247`, `D248`, `D249`
- Issue: `#152` (with `#153`–`#156`, `#158`, `#159`, `#161`, `#162`, `#163`,
  `#442`, `#515`, `#101`)
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
  span before the ID, keys in the order `when start due block rrule
  recur_from remind p labels list done src gcal`. Unknown keys are kept.
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

### `D247`: the web app reschedules by drag and by a Move menu

- A checklist task drags onto a section or onto a bar of every place it
  can go (Inbox, Today, This evening, Tomorrow, In a week, Anytime,
  Someday); the drop calls `tasks.move`. Done is not a drop target.
- Each movable row also has a Move menu, the keyboard and screen-reader
  way. Issues, comments and mentions do not move.
- Rows set or clear a time block; a Calendar panel connects, syncs and
  disconnects Google. A Lists panel creates lists and shares them by
  handle; the owner of the Inbox list is the signed-in person.
- Why: drag alone fails WCAG 2.5.7 and keyboards.

### `D248`: one quick-add dialog for the whole app

- `Q` opens it anywhere. A selection in a page, outside code, opens it
  with the selected text: `⌘⌥↵` or a toolbar button while editing, and a
  bare `T` only on a page that cannot be edited.
- The task's title gets a link back to the page through `tasks.update`
  after parsing, so page words never read as dates.
- The line is parsed with `tasks.parse` 120 ms after typing stops. The
  marked phrases and a list of what they set sit under a plain input.
- Issue comments take an optional "Assign to", sent as `assign_to`.
- Why: a styled overlay inside the field breaks input methods and screen
  readers.

### `D249`: ⌘K searches page titles in memory

- The ⌘K menu matches titles against the page tree the app already holds,
  so it needs no request: under 20 ms for 20,000 pages, timed as
  `ui.search.titles`. Pages come first, then "search every page", then
  commands.
- The search page marks matched words, using the server's fragments when
  it sends them. Owner, modified, location and starred filters live in the
  URL; the server gets those it supports and the app checks all of them.
- Why: a network round trip cannot meet the 20 ms budget (R-SR-1).

## Alternatives

- **A sidecar index for fields**: breaks "files are the truth". Rejected.
- **Dataview-style inline fields**: collide with prose. Rejected.
- **`VTODO` items in the subscribed feed**: most calendar apps ignore them.
- **Tombstones for CalDAV sync**: need storage the crate does not own.
- **Writing to the person's main Google calendar**: needs full calendar
  access. Rejected for a `calendar.app.created` calendar of our own.

## Consequences

- Editors, scripts, CalDAV clients and imports all make the same edits.
- The server binary implements `Workspace` (`#442`). Lists are pages in a
  `Tasks` folder in the owner's My Drive. Sharing grants editor access to
  that one page through the permission engine.
- Checklists in documents count when they mention the person, among the
  200 most recently changed pages they can open.
- The feed, CalDAV and capture routes are mounted cross-site, so a browser
  cookie never counts there. CalDAV signs in with an API token, sent as a
  bearer token or as the HTTP Basic password.
- Time blocks (`#162`) are `block=YYYY-MM-DDTHH:MM/minutes`, synced every
  five minutes with a "Vellum" Google calendar that the app creates. The
  event ID is kept as `gcal=`; a move in the calendar wins over the file.
  The server needs `VELLUM_GOOGLE_CLIENT_ID`, `VELLUM_GOOGLE_CLIENT_SECRET`
  and the redirect URI `<public URL>/connect/google/callback`.
- Non-English quick-add is a follow-up.

In the web app (D247-D249), a typed `T` still replaces a selection while
editing; taking the plain key there would feel like a bug. Title search covers
the pages in the person's spaces; shared-with-me items join when the tree holds
them.
