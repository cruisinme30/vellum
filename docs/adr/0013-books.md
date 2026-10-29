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

The brief (section 7) and D14 make a book a `name.book/` folder: a
`book.yaml` for Vellum settings and a `SUMMARY.md` in mdBook syntax that
builds with unmodified mdBook. Numbers are computed, never stored;
cross-references by ID render as "§2.3 Title"; subtrees move and copy
between books; headings convert to sections and back; snapshots carry
tags and diffs; sections narrow permissions; books publish as static
sites. This record settles the details the brief leaves open. The code is
the `vellum-books` crate (engine layer).

## Decisions

### D67: the book folder

```text
handbook.book/
  book.yaml                  # id, title, numbering, max_depth, export_profile, nodes
  SUMMARY.md                 # order and nesting, mdBook syntax
  book.toml                  # generated from book.yaml
  README.md                  # introduction (a prefix chapter)
  onboarding/README.md       # a section with children
  onboarding/first-week.md   # a leaf section
  .snapshots/v1.0.json       # a tagged snapshot
```

- `book.yaml` has `vellum: 1`, `id`, `title`, `numbering` (`decimal`,
  `outline` or `none`; `"1.1"` is read as `decimal`), `max_depth` (1 to
  6, default 3), an optional `export_profile`, and `nodes`: per-section
  settings **keyed by section ID**, each with its `path` and optional
  `owners`, `reviewers` and `restrict_to`.
- Every chapter file keeps Vellum front matter (`id`, `title`,
  `vellum: 1`). The front-matter ID is the identity (brief section 4); the
  manifest only mirrors where each ID lives.
- `SUMMARY.md` is parsed with `pulldown-cmark`, the parser mdBook itself
  uses, so both read a summary the same way. Vellum writes it back in one
  canonical form that parses to the same tree.
- Vellum writes a `book.toml` (`src = "."`, `build-dir = ".mdbook"`) on
  every save, so `mdbook build` works on the folder as it is. The folder
  mirrors the tree: a section with children is
  `<slug>/README.md` with its children beside it, a leaf is `<slug>.md`.
  When a section gains or loses children, or moves, its files move.
- Chapter file names are lowercase slugs (`first-week.md`), an exception
  to D106, because the paths become public page addresses.
- Numbering follows mdBook: only numbered chapters get numbers, drafts
  count, parts and separators do not, numbering continues across parts.
  `outline` style writes `II.C.4.a.iv.6`.

Paths in `SUMMARY.md` and `book.yaml` are checked: relative, no `..`, no
hidden segments, no symbolic links followed, `.md` only. A hostile
summary cannot read or write outside its book.

### D68: links and structure edits

- `vellum://doc/<id>` is the canonical link and never needs rewriting.
  A link with empty text or `§` is a cross-reference, shown as the target's
  label (`§2.3 Services`) when the page renders or exports, so labels
  follow every reorder without touching files.
- When files move, relative links to and from them are rewritten so they
  keep pointing at the same files.
- Moving a subtree keeps its IDs. Relative links that would cross a book
  boundary become ID links on both sides. Copying gives every copied
  section a fresh ID and points links inside the copy at the copies.
  Images and attachments the subtree links to are reported for the caller
  to copy.
- Splitting a section at its headings creates one child per heading;
  merging folds children back as headings (depth gives the level, capped
  at 6). Links to `parent#anchor` follow the split; links to merged
  sections follow the merge; the returned redirects let the index fix
  links from outside the book.
- Loading repairs edits made outside Vellum: a moved file is found by
  its ID, a file without an ID gets one, a second file with the same ID
  gets a fresh one, and a summary entry whose file is missing gets a new
  draft page (R-BK-7). Each repair is reported.

### D69: snapshots, access and publishing

- A snapshot is one immutable JSON file, `.snapshots/<tag>.json`, holding
  the manifest, the summary and every chapter. It travels with the book
  through Git and backups and does not depend on version retention.
  Per-file history stays with the version service (R-BK-2). A diff matches
  sections by ID and reports added, removed, moved or given a new number, and
  changed sections with unified patches.
- Section restrictions only narrow. A section is visible when the viewer
  is in the `restrict_to` set of that section and of every enclosing
  section that has one. A child cannot name someone its parent excludes.
  Owners and reviewers come from the nearest section that names any. The
  authorization service remains the source of truth and reads these
  settings into its tuples.
- Publishing leaves out every section the audience cannot see, titles
  included, without moving other files, so page addresses stay stable.
  Cross-references become labels, ID links become relative links, links to
  left-out sections become plain text.
- Two outputs: an mdBook source tree, and a static HTML site rendered
  through the render service's `Renderer` port, with numbered navigation,
  previous and next links, `toc.json`, a stylesheet and no scripts.

## Alternatives

- **Store numbers in files**: every reorder would rewrite many files.
- **A Vellum-only manifest without `SUMMARY.md`**: plain mdBook could not
  build the book.
- **Snapshots in version history**: they would depend on retention and
  not travel with the folder.

### D143: the books service

- A new crate, `vellum-books-service` (service layer), registers the
  `books` service on the service seam: list, table of contents, read a
  section, add, move, update and remove sections, set section access,
  and snapshots with diffs, under `/v1/books/...`.
- Each request loads the book from the blob store, so edits made on disk
  are seen at once. A save writes only changed files, guarded by the
  etags read, with `book.yaml` checked first, so an edit made meanwhile
  fails with a conflict and is never overwritten.
- Removing a section deletes its files; snapshots and the store's
  history keep earlier versions.

### D144: section access is enforced and mirrored

- Opening a book needs read access to its `.book` node, editing needs
  edit access, and changing section access needs manage access.
- A restricted section is visible to book managers and owners, and to
  others only through one of its effective `restrict_to` principals
  (their user or one of their groups). Anything else answers "not
  found". Removing a section needs sight of everything under it.
- A principal is `user:<UserId>` or `group:<GroupId>`, with the ID as the
  kernel ULID in upper case. Matching is exact.
- After every edit, the service mirrors each section's effective
  restriction into authorization: a `restricted` marker plus an `editor`
  grant per principal, which the engine caps at the principal's relation
  on the book. The index already registers sections under the book, and
  the effective set is a literal intersection, so a flat tree is exact.
  Sections the engine does not know yet are skipped; the service still
  enforces them.
- Every role grant on a section node belongs to the mirror, because the
  drive never shares a chapter on its own.

### D145: one numbering for every reader

- Readers who cannot see a section get the book's own numbers, so
  colleagues quote the same `§2.3`. Hidden sections drop out of the
  contents, and a cross-reference to one keeps its bare `§`.
- A gap in the numbers shows that something is hidden, never what.
- Diffs leave out sections hidden now (or, for sections since removed,
  hidden in the version that had them), and show settings and summary
  changes to managers only.
- Copying a whole book in the drive copies only what the copier may see
  (`Book::view_for`) and never copies `.snapshots/`, because snapshots
  hold every section's text (red-team finding #327).

## Alternatives

- **Enforce sections only in the books service**: search, MCP and export
  would still see restricted chapters through the index.
- **Renumber per reader**: numbers would differ between colleagues and
  cross-references would break when quoted.
- **Nest sections in the authorization tree**: needs the index and the
  drive to understand `SUMMARY.md`; the literal intersection makes it
  unnecessary.

## Consequences

- The export service walks `Book::toc()`: each entry has its number,
  label, depth and heading level, and `TocEntry::rebased` starts a
  section's export at level 1 (R-BK-5).
- The HTTP and GraphQL operations for books come from the books service
  (D143), not the pages service.
- The watcher should call `BooksService::sync_access` when a `book.yaml`
  changes on disk, so outside edits to restrictions reach authorization.
- The drive watcher must ignore `.mdbook/` and treat `.snapshots/` as book
  data, not documents.
