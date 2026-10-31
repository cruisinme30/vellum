---
id: 01M66X8TDRBW31GRD4F9V7N0BX
title: "ADR 0075: Creating books"
status: draft
owner: "@cruisinme30"
tags: [adr, books, drive, web]
vellum: 1
---

::: facts

- Date: 2026-10-30
- Status: accepted
- Decisions: D322, D323
- Issue: #566
- Pull requests: to be linked when merged

:::

## Context

The user guide and three cookbook recipes start with "Create a book", but
nothing could: the books API edits books that exist, the drive made only
folders, pages and shortcuts, and New made a page. A book is a
`Name.book/` folder holding `book.yaml` and `SUMMARY.md`. Its
permissions follow the drive's folder tree, so a book written to the store
without the drive is hidden from everyone, its creator included (the same
fault as #565, D321).

## Decisions

### `D322`: the drive makes a book; `books.create` asks it to

- `Drive::create_book` takes a drive folder (a space's root folder or a
  live folder, as D305) and a title. It checks edit rights there, writes
  `book.yaml` and `SUMMARY.md` for an empty book at a free
  `Title.book` path, and indexes it with its creator as owner.
- `POST /v1/books` (`books.create`, `write:direct`) takes
  `{folder, title}` and returns the book's ID, title and folder. The books
  service reaches the drive through its `Shelf` port, which
  `vellum serve` provides; without one it refuses.
- The books service adds the new book to the book list it keeps, so the
  list shows it at once.

### `D323`: New offers a book where books can be made

- With a server, **New** in the rail and the bottom bar is a menu button
  offering **Page** and **Book**. In the demo, which has no books, it
  still makes a page at once.
- **Book**, and **New book** on the Books screen, open a form for the
  title and the drive (My Drive first, then the shared drives). Creating
  opens the empty book.

## Alternatives

- **The books service writes the files**: it would have to undo the write
  when the drive refuses, and check folder rights itself before writing.
- **Only a Books screen button**: the guide's "New → Book" would stay
  wrong, and New would keep hiding that books exist.
- **Any folder in the form**: the web app holds no folder paths yet, so
  the form offers drives; a folder picker can come later.

## Consequences

- Books join folders and pages as things the drive makes, with one
  permission check and one owner grant.
- New is one more click for a page on a server; the demo is unchanged.
