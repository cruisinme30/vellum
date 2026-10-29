---
id: 01M63CFE6EE72DZXNSS697V5HS
title: The drive
vellum: 1
---

# The drive

The drive organises everything in Vellum: pages, books, collections,
task lists and the files attached to them. What you see in the sidebar
is the folder on disk, or in S3, Google Drive, Dropbox, OneDrive or Git.

## Places {#places}

::: facts

- My Drive: your own pages and folders
- Shared drives: folders a team owns together
- Shared with me: things other people shared with you directly
- Starred: things you starred, for quick access
- Recent: things you opened or changed lately
- Trash: deleted things, kept for 30 days

:::

## On disk {#layout}

```text
northstar/
  .vellum/                    # index, trash and keys, never content
  drives/shared/product/
    .vellum-folder.yaml
    Roadmap 2027.md
    handbook.book/
    requirements.collection/
  drives/users/priya/
  teams/ENG/issues/
  tasks/priya/
```

A page's identity is the `id` in its front matter, never its path, so
you can rename or move anything without breaking links.

## Move, copy and shortcuts {#move}

Drag to move. A moved page keeps the access that was granted on the page
itself and takes the access of its new folder. **Make a copy** creates
new pages with new IDs; copying a book copies every section.

A **shortcut** puts a link to a page in another folder. It opens the
original and never grants access to it.

```yaml
# Roadmap 2027.shortcut.yaml
to: 01JB3K9Q7Z5N8M2X4V6T0R1Y3W
```

## Trash {#trash}

Deleting a folder moves it and everything in it to the trash. Restoring
the folder restores what was deleted with it, but not what you had
deleted separately before. Trash empties itself after 30 days, or a
period your admin sets per drive.

## Search {#search}

Press `⌘K` and type. Filter by owner, last modified, type (page, book,
collection, issue, task, file), location and starred. Search reads
inside pages, books, rows and attachments, and only ever shows what you
may open.

## Activity and backlinks {#activity}

Each page's **Activity** tab lists who changed it and when. **Backlinks**
lists every page that links to it.
