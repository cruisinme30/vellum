---
id: 01M64TV2D9AA72HEJHRZ93B84H
title: "ADR 0052: Drive web UI"
status: verified
owner: "@cruisinme30"
tags: [adr, drive, web]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D226, D227, D228, D266
- Issue: #503, #509
- Pull requests: to be linked when merged

:::

## Context

The drive's server side is complete (ADR 0012, ADR 0039): `vellum serve`
answers every `drive.*` operation, from My Drive and the shared drives to
trash, activity, backlinks, search filters, usage and space settings.
The web app used only a few of them, to load the page tree. A person had
no way to see Shared with me, the trash, an item's history or a space's
quota, and no way to move or copy an item to another drive. That covers
R-DR-1, R-DR-3, R-DR-4, R-DR-7, R-DR-9 and R-DR-10 of the PRD. ADR 0012
records the drive itself and has no room left, so the web side gets its
own record.

## Decisions

### D226: the Drive is a server-only area with its own routes

- The Drive lives under `/drive`: My Drive at `/drive`, then
  `/drive/shared`, `/drive/shared-with-me`, `/drive/starred`,
  `/drive/recent` and `/drive/search`.
- A space is `/drive/s/{space}`, a folder `/drive/s/{space}/f/{folder}`,
  and each space has `/trash` and `/settings` beneath it.
- `DriveApi` (web/src/drive/api.ts) is one more entry in `ServerApis`,
  like Work and Books. In demo mode the screens say they need a server.
- The rail stays as the brief fixes it. Home links to the Drive, and the
  Drive's own tabs move between its places.
- An item opens where it lives: a page in the editor (`/p/{id}`), a book
  in the reader, a collection in its views and a folder in the Drive. A
  shortcut asks the server for its target and opens that.

### D227: one dialog places items, and deleting for good asks twice

- Move, Copy and Shortcut share one dialog. It picks a drive (My Drive
  first, then the shared drives) and a folder or page from that drive's
  tree, loaded in one `drive.tree` request.
- Moving never offers the item itself or anything under it. The server
  still checks, and its refusal shows in the dialog.
- Trash lists the space's trashed items with the day each goes for good,
  from the space's trash days. Restore is one click.
- Delete for good and Empty trash take a second click on a button that
  names what goes, because the server cannot undo them.
- A list keeps its rows and its last message on screen while it reloads
  after a change, so a notice is not lost to the reload.

### D228: details, filters and settings use the server's own shapes

- Details opens a side panel with the item's facts, its activity feed
  and, for a page, the pages linking to it. A space's own feed comes from
  `drive.activity` with the space's ID.
- Activity names the signed-in person "You". Others show as "Someone"
  until the app has a people directory to resolve user IDs.
- Search filters (title, kind, modified range, drive, starred) live in
  the URL, so a search can be shared and the back button works. The day
  range is turned into instants in the person's own time zone.
- The usage meter warns from 90 % of the quota. Settings edit the quota
  in GB, the trash days and the version retention, and "Apply retention
  now" runs `drive.prune_versions`.

### D266: trash expiry and version retention run once per cluster

- `vellum serve` runs `Drive::purge_expired` every hour and
  `Drive::prune_all_versions` every day, each under its own lease
  (`drive.trash_expiry`, `drive.version_retention`), so one replica
  does the work, as for the other cluster jobs (ADR 0038).
- Both act as Vellum itself and need no person's rights. Each space
  keeps its own trash days and retention policy.
- A replica that takes a lease runs the job at once, so a restart
  catches up. "Apply retention now" stays as a manual trigger.

## Alternatives

- **Fold the Drive into the sidebar's page tree**: the tree shows one
  space's pages for writing. Trash, quotas and Shared with me do not fit
  there, and the brief keeps the rail fixed.
- **A drag-and-drop move in the list**: it cannot cross drives, and it is
  hard to do with a keyboard. The dialog works for both; drag can come
  later on top of the same calls.
- **Browser `confirm()` before deleting**: it blocks the page, cannot be
  styled or translated, and some embedded browsers turn it off.
- **Keep filters in component state**: a reload or a shared link would
  lose the search.

## Consequences

- Every R-DR navigation, trash, activity, filter and quota requirement
  now has a screen. Uploads stay with the upload route (ADR 0039 D211-D213).
- Each list reads the person's stars in a second request.
- Trash and old versions now go on their own (#509). Retention reads
  every page's versions once a day, which grows with the workspace.
- Activity shows no names for other people until a people directory
  exists.
