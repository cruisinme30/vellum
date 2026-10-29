---
id: 01M63CXX0ASVR2DTKG34E6QABJ
title: "ADR 0012: The drive service"
status: verified
owner: "@cruisinme30"
tags: [adr, drive]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D64, D65, D66
- Issue: #103, #104, #105, #106, #107, #108, #109
- Pull requests: the drive service pull request

:::

## Context

The drive is the organisation layer from brief section 6 and
requirements R-DR-1 to R-DR-10: My Drive, Shared drives, Shared with me,
Starred, Recent and Trash; permission inheritance; move, copy and
shortcuts; link rewriting on rename; activity and backlinks; quotas and
version retention. What you see is the disk, the index is rebuildable,
and the store (ADR 0005, ADR 0026) already gives atomic conditional
writes, the three-way merge and an identity-based watcher.

## Options considered

- **Put the drive in `vellum-pages`**: the brief lists that crate for
  spaces and pages, but the books thread works there too, and two
  services in one crate break the one-service-per-crate rule.
- **A `vellum-drive` crate with its own ports**: chosen. It depends on
  the store and on `vellum-authz` and owns nothing else's tables.
- **Trash as a flag on files left in place**: rejected, because trashed
  content would stay visible to Git, sync clients and the file browser.
- **Trash as a move into `.vellum/trash/`**: chosen, as the PRD layout
  shows.

## Decisions

### D64: the drive is a service crate over three ports

- `vellum-drive` (layer `service`) holds `Drive`. It reaches files
  through `BlobStore`, the index through `DriveIndex` and authorization
  through `Access`. `MemoryIndex` is the reference index; `AuthzAccess`
  runs `Access` on `vellum-authz`, handing stores that keep their own
  tree (`TreeSync`) the drive's parent links before re-materialising.
- Every operation checks permission first (deny by default), writes the
  files, then the index, then appends activity. A crash in between
  leaves files ahead of the index, and `scan` repairs it.
- A model acting for a person gets at most that person's view and edit
  rights. Purging, settings and leaving a shared drive need manager
  rights, which models never get.
- Space settings (quota, trash days, retention) live in the space's
  root `.vellum-folder.yaml`, so `scan` rebuilds spaces, nodes, trash,
  versions and links from files alone. Stars, recents and activity are
  database-owned.

### D65: trash, versions and quotas

- Trashing moves a node's files to `.vellum/trash/<id>/item/` and writes
  a manifest listing the node and every node that went with it. Only
  the node itself is `explicitly_trashed`. A node trashed on its own
  earlier sits in its own entry, so restoring its folder later leaves
  it in the trash: restore is exact. Members restore through their
  entry. A restored node whose folder is gone lands at the space root;
  a taken name gets a ` (2)` suffix.
- Entries older than the space's `trash_days` (default 30) are purged by
  `purge_expired`. Purge removes files, versions and index rows.
- Every content write stores a version at
  `.vellum/versions/<doc>/<version>.md` with a YAML sidecar naming the
  author. Version IDs carry the drive clock's time. Retention keeps
  every version for `keep_all_days` (30), then the newest per day for
  `daily_days` (90), then the newest per ISO week for `weekly_weeks`
  (forever by default); the newest version always stays.
- A quota counts live content, trash and versions. A write that would
  exceed it fails with `QuotaExceeded`.

### D66: links, copies and shortcuts

- Stored links are `vellum://doc/<id>` and survive everything. Renames
  and moves rewrite relative links and wikilinks: links inside moved
  pages that point elsewhere, and links in other pages that point at
  moved or renamed pages. A wikilink is renamed only when no other live
  page still has the old title. Front matter and code are never touched.
- A copy maps every ID found in the copied files (front matter, folder
  metadata, `book.yaml`, schemas, snapshots) to a fresh ULID and
  substitutes it across the copy, so links inside the copy point inside
  the copy. The copier owns the copy and no grants are copied.
- A shortcut is `Name.shortcut.yaml` holding its own ID and the
  target's. Creating one needs view rights on the target; following one
  checks the target again, so a shortcut never grants access.
- External edits reported by the watcher become versions authored by
  `External`; tree changes rescan the touched spaces, where the node
  follows its ID and a duplicate ID goes to the newcomer.

## Consequences

- The SQL `DriveIndex` adapter follows `MemoryIndex` and the end-to-end
  tests once the index schema for activity, links and versions lands in
  `vellum-db`.
- The REST and GraphQL operations follow the service seam from
  `vellum-contracts` and map one-to-one to `Drive` methods.
- Listing a large space reads every node of it from the index; the SQL
  adapter answers those with indexed queries.

## History

- 2026-10-29: accepted.
