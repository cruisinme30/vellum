---
id: 01M64DP6CE6RVR5TZ6XJ217H77
title: "ADR 0039: Drive page tree and serve gaps"
status: verified
owner: "@cruisinme30"
tags: [adr, drive]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D168, D169, D170, D171
- Issue: #448, #449, #450, #451
- Pull requests: to be linked when merged

:::

## Context

Wiring the web app to the server's drive (#392) showed what the server
was missing. Pages could sit only in folders, the drive kept no order
for siblings, the tree took one request per folder, and a new account
had no My Drive until a client made one. Search in `vellum serve`, the
fifth gap, is #429 and lands in #465. The drive's rule from ADR 0012
still holds: what you see is the disk, and the index can be rebuilt
from the files alone.

## Decisions

### D168: a page holds its nested pages in the directory beside it

- A page `Plan.md` keeps the items nested under it in `Plan/` beside it.
  That directory has no `.vellum-folder.yaml`, so it is not a folder.
- When `Plan/` has a folder file (made outside Vellum), it is a folder
  and the page holds nothing.
- A page named "X" needs both `X.md` and `X/` free, and so does a folder.
  A page and a folder therefore never share a directory.
- Move, copy, rename, trash, restore and a fresh scan carry the nested items
  with the page. Moving a page under itself is refused.
- A nested item's parent is the page, so its access inherits through the
  page. In the shared `documents` table its folder is left empty.

### D169: sibling order lives in an order file

- `.vellum-order.yaml` in a container's directory (the space root, a
  folder, or a page's directory) lists child IDs in order.
- Items it doesn't list come after, folders first, then by title. IDs
  that are no longer children are ignored.
- `drive.move` and `drive.copy` take an `index` among the new siblings.
  Reordering rewrites only the order file, never the pages.
- The watcher ignores order files: listings read them, so nothing in the
  index goes stale.

### D170: the whole tree in one request

- `GET /v1/drive/spaces/{space_id}/tree` returns the live items the
  caller may open: depth first, siblings in order, each naming its
  parent. It can start under one item and stop at a depth.
- An item the caller can't open hides everything beneath it.
- The limit is 2,000 items by default and 10,000 at most. A cut tree
  says `truncated`.

### D171: Vellum makes each person's My Drive

- `Drive::ensure_my_drive` makes a My Drive named "My Drive", owned by
  the person. Its slug comes from their handle by the web app's rule,
  with the end of their ID added when the slug is taken.
- Calls made at once are safe: a drive just written for the same person
  is found from its folder file and kept.
- `vellum serve` makes the missing drives at startup and every 30
  seconds after. `GET /v1/drive/my-drive` makes one on a first visit.

## Alternatives

- **A page with children becomes `Plan/index.md`**: its path would
  change on its first child, which breaks links and history.
- **An `order` key in each page's front matter**: every reorder would
  write a page version.
- **A position column in the index**: the order would be lost on a
  rebuild and need a migration.
- **An identity hook on user creation**: it would touch every way an
  account is made. A periodic check plus making the drive on first visit
  covers them all.

## Consequences

- Clients can drop their own My Drive creation and the per-folder walk.
- Each listing reads one small order file.
- A tree for a large space checks permission item by item.
- With more than one replica each one checks for missing My Drives; a
  race is safe, and running it once per cluster is #439's work.
