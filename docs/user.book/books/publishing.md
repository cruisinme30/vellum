---
id: 01M63CFE6BM482SF9S81PZXTEG
title: Exporting and publishing a book
vellum: 1
---

# Exporting and publishing a book

## Export {#export}

Open **⋯ → Export** on a section and choose what to include:

::: facts

- This section: only the section you are on
- This section and its children: a chapter with everything below it
- Whole book: every section, in order

:::

Exports map depth to heading levels, regenerate the table of contents
and turn cross-references into page numbers. PDF exports get a cover,
running heads and page numbers; Word exports use Word's own heading
styles, so Word's navigation pane works.

## Publish as a site {#publish}

A book manager opens **Publish** and picks a format:

- **Static website**: pages with a table of contents and previous and
  next links, and no scripts. Published for everyone, Vellum serves it at
  `/published/books/<book id>/`.
- **mdBook source**: a tree that `mdbook build` or the public docs site
  builds.

Both land in the book folder under `.published/`, so a Git-backed space
pushes them with the book. Publish for **Only some people** to keep
their restricted sections; a public site never shows a restricted
section. Readers see the published version until you publish again, so
work in progress never leaks.

## Git Sync {#git-sync}

Keep a book in a Git repository and every change in Vellum becomes a
commit; every push to the repository flows back into Vellum through the
same merge as any other external edit. The
[docs-as-code recipe](../cookbook/docs-as-code.md) walks through it.
