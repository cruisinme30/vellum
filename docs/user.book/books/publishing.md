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

**⋯ → Publish** turns a book into a static website with search, a
table of contents and previous and next links, built with mdBook. Choose
where it goes:

- **The public docs site** of your Vellum, at a path you pick.
- **A custom domain** with its own theme, such as `docs.northstar.dev`.
- **A folder or Git repository**, to host anywhere.

Each publish is a snapshot. Readers see the published version until you
publish again, so work in progress never leaks. The site includes an
`llms.txt` file so AI tools can read it.

## Git Sync {#git-sync}

Keep a book in a Git repository and every change in Vellum becomes a
commit; every push to the repository flows back into Vellum through the
same merge as any other external edit. The
[docs-as-code recipe](../cookbook/docs-as-code.md) walks through it.
