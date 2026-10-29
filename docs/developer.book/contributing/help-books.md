---
id: 01M63CFE66RJXGYW4SR9CQQRYC
title: Maintaining the help books
vellum: 1
---

# Maintaining the help books

The three help books live in `docs/` as Vellum books. They are shipped
inside the binary, published as a website with mdBook and used as test
input, so a change to them is checked like a change to code.

```text
docs/
  user.book/           # the user guide and cookbook
  admin.book/          # the admin guide
  developer.book/      # this guide
  help-anchors.yaml    # in-app help IDs mapped to pages
  adr/                 # architecture decision records
```

Each book has a `book.yaml` (its ID, numbering and a ULID for every
page) and a `SUMMARY.md` in mdBook syntax. Every page starts with front
matter whose `id` matches its entry in `book.yaml`.

## Examples are tests {#examples}

In the user guide, each block example is the block itself followed by
its source in a fence marked for the corpus. Four backticks let the
example contain a three-backtick fence.

`````markdown
::: tip
Press / to insert any block.
:::

````markdown {corpus="container/tip"}
::: tip
Press / to insert any block.
:::
````
`````

::: facts

- `corpus="kind/name"`: the file the example becomes, `corpus/guide/kind/name.md`
- kinds: inline, block, container, fence, mermaid, table, page
- `live=false`: a source-only example, for front matter and whole pages that cannot sit inside a chapter

:::

`cargo xtask help` checks that:

- every live example is identical to the block just above it
- every known block and every Mermaid diagram type has an example
- every `SUMMARY.md` entry exists, and every page is in `SUMMARY.md`
- every page's `id` is a ULID, unique, and matches `book.yaml`
- every entry in `help-anchors.yaml` points at a real heading
- `corpus/guide/` and the embedded copy in `vellum-help` are up to date

`cargo xtask help write` regenerates `corpus/guide/` and the embedded
page table, and gives new pages an ID. Run it after editing the books.

## Contextual help {#anchors}

The `?` button in the app opens the page for the current screen. The
app knows only a help ID such as `editor.slash-menu`;
`docs/help-anchors.yaml` maps each ID to a page and heading:

```yaml
editor.slash-menu: user/start/README.md#slash-menu
```

Give the target heading a custom anchor, `{#slash-menu}`, so that
rewording the heading never breaks the link. Never reuse or rename a
help ID; add a new one and keep the old one pointing somewhere sensible.

## Style {#style}

- Write for someone busy: lead with what the reader can do, in short sentences.
- Name interface elements in bold, exactly as they appear: **Share**, **⋯ → Export**.
- Use the Northstar workspace and its people for every example.
- Vale and markdownlint run in CI; add real words Vale does not know to `.vale/styles/config/vocabularies/Vellum/accept.txt`.

## Keeping the books current {#current}

A pull request that changes something users, admins or developers can
see updates the matching chapter in the same pull request. Each release
ships its books inside the binary, so `vellum docs` always matches the
version that is running.

## Videos {#videos}

Short videos cover the ten most common tasks, one per task, each under
two minutes and captioned. They are recorded against the Northstar demo
workspace from the matching cookbook recipe or chapter:

1. Create and share a page
2. Use the slash menu
3. Add a chart and a diagram
4. Create a collection and a board view
5. Create a book and reorder its sections
6. Export to PDF and Word
7. Create and triage issues
8. Quick-add tasks and subscribe to the calendar feed
9. Connect Claude over MCP and review a draft
10. Restore a page from version history

## Translations {#translations}

Interface strings are translated with Fluent. The books are translated
as copies under `docs/i18n/<locale>/`, keeping the same paths and page
IDs, so a help ID resolves in every language and falls back to English
where a page is not yet translated.
