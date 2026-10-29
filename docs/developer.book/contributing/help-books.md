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

`cargo xtask help write` also writes `web/src/help/topics.gen.ts`, so the
app can only name IDs that exist. Put a `?` beside a screen's title with
`<ViewTitle title={…} help="mywork.buckets" />`, or pass `help` to
`ServerScreen` or `AdminScreen`. Against a server the link opens
`/guide/topics/<id>`, the books built into that server, rendered by
`vellum-render` like any page; in the demo it opens the published guide.

## The website {#website}

`scripts/ci/build-site.sh` publishes the books with mdBook. Every page
ends with **Was this page helpful?** and **Edit this page**: Yes and No
open a prefilled GitHub issue labelled `docs-feedback`, and nothing is
sent unless the reader submits it. The newest five release tags get their
own copy at `docs/v/<tag>/`, linked from the site bar, so readers of an
older release find its guide.

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

Ten short videos cover the most common tasks, each under two minutes and
captioned. They are recorded, not filmed: a tour in `web/e2e/tours`
drives the app against the Northstar demo workspace, and `pnpm run
tours` writes each video and its WebVTT captions to
`web/test-results/tours`. The [video storyboard](videos.md) has the
scenes and captions of all ten. Re-record them for each release, so a
video never shows an older screen than the release it ships with.

## Translations {#translations}

Interface strings are translated with Fluent. The books are translated
as copies under `docs/i18n/<locale>/<name>.book/`, such as
`docs/i18n/de/user.book/`. A translation:

- keeps every page's path and `id`, and the book's `id`;
- keeps every heading's `{#anchor}`, so help IDs resolve in every language;
- may translate only some pages; the rest fall back to English;
- translates text only: examples, code and Northstar names stay as they are.

`cargo xtask help` checks all four. Keep text out of screenshots and
diagrams where you can, so they need no translated copy.
