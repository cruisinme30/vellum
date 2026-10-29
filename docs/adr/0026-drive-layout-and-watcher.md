---
id: 01M63BF7TFRV3BD9SV17655C29
title: "ADR 0026: Drive layout, file names and the external-change watcher"
status: verified
owner: "@cruisinme30"
tags: [adr, storage, foundation]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D106, D107, D108
- Issue: none (recorded before the issue rule, D119)
- Pull requests: #37

:::

## Context

The brief (section 6) fixes the on-disk layout and says identity is the
ULID, never the path: a rename made outside Vellum is detected by ID.
The brief also calls file names "title slugs" without saying what a slug
keeps. This record settles file naming and how the watcher turns raw
file system notifications into changes by identity.

## Decisions

### D106: file names are readable titles, made portable

- A file name is the title as people typed it, not a lowercase slug, so
  the folder reads like the app. Characters that Windows, macOS, S3 or
  Google Drive reject (`/ \ : * ? " < > |` and control characters)
  become `-`. Leading dots and trailing dots or spaces are trimmed.
  Windows device names (`CON`, `COM1`, …) get a suffix. Names are cut on
  a character boundary to 200 bytes.

### D107: the watcher reports changes by identity

- A change feed reports touched paths only. The watcher re-reads them
  and reads the `id` from front matter (Markdown) or the top-level `id`
  key (`book.yaml`, `.vellum-folder.yaml`, collection schemas,
  shortcuts).
- Same id at a new path while the old path is gone: **renamed**. Same id
  while the old file still holds it: **duplicated**; the copy must get a
  fresh id. Unknown id: **created**. New content at a known path:
  **modified**. Files without an id are reported as untracked.
- A removal waits two seconds for the other half of a rename, because
  the two halves can land in different batches.
- A feed reports a moved folder as the folder alone, so a touched path
  also stands for every file known or now present beneath it.
- Batches end after 300 ms of quiet, since editors save in bursts. Writes
  Vellum makes itself are recorded with the watcher, so their echo is
  silent.
- `.vellum/`, `.git/`, temp files and editor scratch files (`.swp`, `~`,
  `.DS_Store`, …) are never ingested.

### D108: startup compares the whole drive with the index

- The watcher starts from what the index knows (id, path, version) and
  scans the store once. Edits, renames and deletes made while Vellum was
  down come out as the same changes as live ones. Deletes found this way
  are reported at once.
- The same reconcile code serves local folders now and S3 events or
  cloud-drive change feeds later; only the feed differs.

## Alternatives

- **Lowercase slugs**: the folder would not read like the app.
- **Identity by path**: an outside rename would look like a delete and a
  create, losing history and links.
- **Trust the feed's rename events**: feeds split, merge or drop them.

## Consequences

- Conflict copies read naturally, and the same tree works on every
  backend.

- The index applies changes in order and calls the three-way merge from
  ADR 0005 when a modified document has a live editing session.
- The watcher holds one entry per object in memory, about 150 bytes
  each, which fits the idle memory budget up to several hundred thousand
  objects. A larger drive would keep this map in the index instead.
