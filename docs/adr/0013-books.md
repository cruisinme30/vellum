---
id: 01M63CPCJZSV7F709M97XBQSCE
title: "ADR 0013: Books"
status: verified
owner: "@cruisinme30"
tags: [adr, books, drive-books]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D67, D68, D69, D143, D144, D145
- Requirements: R-BK-1 to R-BK-10
- Issue: #112, #113, #114, #115, #116, #117, #118, #327, #336
- Pull requests: #212

:::

## Context

The brief (section 7) and D14 make a book a `name.book/` folder with a
`book.yaml` and an mdBook `SUMMARY.md` that unmodified mdBook builds.
Numbers are computed, cross-references by ID read "§2.3 Title", subtrees
move and copy, headings convert to sections, snapshots carry tags,
sections narrow access, and books publish as sites. The code is
`vellum-books` (engine) and `vellum-books-service` (service).

## Decisions

### D67: the book folder

- `book.yaml`: `vellum: 1`, `id`, `title`, `numbering` (`decimal`,
  `outline`, `none`), `max_depth` (1 to 6, default 3), `export_profile`,
  and `nodes` keyed by section ID with `path`, `owners`, `reviewers` and
  `restrict_to`. The front-matter `id` of each chapter is its identity.
- `SUMMARY.md` is parsed with `pulldown-cmark`, mdBook's own parser, and
  written back in one canonical form. A generated `book.toml`
  (`src = "."`, `build-dir = ".mdbook"`) lets `mdbook build` run as is.
- The folder mirrors the tree: a parent is `<slug>/README.md`, a leaf
  `<slug>.md`. Slug names are an exception to D106 because they become
  page addresses.
- Numbering follows mdBook; `outline` writes `II.C.4.a.iv.6`.
- Paths are relative, without `..`, hidden segments or symbolic links,
  so a hostile summary cannot reach outside its book.

### D68: links and structure edits

- `vellum://doc/<id>` is canonical. Empty or `§` link text is a
  cross-reference, labelled when shown, so labels follow every reorder.
- Moving files rewrites relative links to and from them. Moves keep IDs;
  copies get fresh IDs with internal links re-pointed; links across books
  become ID links.
- Splitting at headings and merging back return redirects for the index.
- Loading repairs outside edits (moved files, missing or duplicate IDs,
  missing files become drafts) and reports each repair.

### D69: snapshots and publishing

- A snapshot is one immutable `.snapshots/<tag>.json` with the manifest,
  summary and every chapter. A diff matches sections by ID. Per-file
  history stays with the version service (R-BK-2).
- Restrictions only narrow: the effective `restrict_to` is the
  intersection of every enclosing one, and a child cannot widen it.
  Owners and reviewers come from the nearest section naming any.
- Publishing leaves out every section the audience cannot see and emits
  an mdBook tree or a script-free static site.

### D143: the books service

- `vellum-books-service` registers `books` on the service seam: list,
  contents, read a section, add, move, update and remove sections, set
  section access, snapshots and diffs, under `/v1/books/...`.
- Each request loads the book from the store, so disk edits show at once.
  A save writes only changed files under the etags read, `book.yaml`
  checked first, so a concurrent edit fails with a conflict.
- Removing a section deletes its files; snapshots and store history keep
  older versions.

### D144: section access is enforced and mirrored

- Reading needs read access to the `.book` node, editing needs edit,
  changing section access needs manage.
- Book managers and owners see every section; others see a restricted
  one only through an effective `restrict_to` principal (their user or a
  group). Hidden sections answer "not found"; removing a section needs
  sight of everything under it.
- Principals are `user:<UserId>` or `group:<GroupId>` with the kernel
  ULID in upper case, matched exactly.
- After each edit the service mirrors every effective restriction into
  authorization: a `restricted` marker plus an `editor` grant per
  principal, capped by the engine at the principal's role on the book.
  The index registers sections flat under the book, which is exact for a
  literal intersection. Sections not yet indexed are skipped and still
  enforced by the service.
- Role grants on section nodes belong to the mirror: the drive never
  shares a chapter on its own.

### D145: one numbering for every reader

- Readers keep the book's own numbers, so colleagues quote the same
  `§2.3`; a gap shows that something is hidden, never what. Hidden
  sections leave the contents, and cross-references to them stay `§`.
- Diffs drop sections hidden now (or hidden in the version that had
  them) and show settings and summary changes to managers only.
- A drive copy of a book holds only what the copier may see
  (`Book::view_for`) and never `.snapshots/` (#327).

## Alternatives

- **Numbers stored in files**: every reorder would rewrite many files.
- **No `SUMMARY.md`**: plain mdBook could not build the book.
- **Snapshots in version history**: they would depend on retention.
- **Enforce sections in the service only**: search, MCP and export would
  still reach restricted chapters through the index.
- **Renumber per reader**: quoted numbers would differ between readers.

## Consequences

- Export walks `Book::toc()`; `TocEntry::rebased` starts a section export
  at level 1 (R-BK-5).
- The watcher should call `BooksService::sync_access` when `book.yaml`
  changes on disk, ignore `.mdbook/`, and treat `.snapshots/` as data.
- `vellum-cli` registers `BooksService` with the other services.
