---
id: 01M63CFE67FTFQ8XDH4S7ET305
title: Writing in Vellum Markdown
vellum: 1
---

# Writing in Vellum Markdown

Every Vellum page is a `.md` file written in **Vellum Markdown**:
CommonMark and GitHub Flavored Markdown, plus YAML front matter, `:::`
containers, special code fences for charts and diagrams, and `$$` math.
There is no raw HTML, ever. Standard Markdown renders everywhere; Vellum
blocks render richly in Vellum and stay readable text anywhere else.

You rarely type the Markdown yourself: the slash menu and the toolbar
write it for you. This part of the guide shows it anyway, so you know
exactly what is stored and can write pages in any text editor.

## How examples are shown

Each example shows the block as Vellum renders it, followed by the
Markdown that makes it. For example, this is an info callout:

::: info
The gateway is the single entry point for every request.
:::

````markdown {corpus="container/info"}
::: info
The gateway is the single entry point for every request.
:::
````

The examples in this part are also Vellum's test suite: each one is
parsed, rendered and exported on every change to Vellum, and the build
fails if the two copies above ever disagree.

## Front matter {#front-matter}

Every page starts with front matter: a short YAML block between `---`
lines. The editor writes it for you and shows it as the page header.

````markdown {corpus="page/front-matter" live=false}
---
id: 01JB3K9Q7Z5N8M2X4V6T0R1Y3W
title: Roadmap 2027
icon: 🗓
status: verified
owner: priya
tags: [planning]
verify_every: 90d
vellum: 1
---
````

::: facts

- `id`: the page's identity, a ULID; it never changes, so you can rename or move the page freely
- `title`: the page title, shown in the tree and in links
- `icon` and `cover`: an emoji or image for the header
- `status`: draft, in-review, verified or outdated
- `owner`: who verifies the page and gets its reminders
- `tags`: a list of tags for search and filters
- `verify_every`: how often a verified page asks to be checked again
- `vellum`: the dialect version, always 1

:::

Everything except `id` is editable. The file name is a slug of the
title; Vellum renames the file when you rename the page and finds it by
`id` if you rename it outside Vellum.

## Attributes {#attributes}

Any block accepts Pandoc-style attributes on its first line: `#id` for
an anchor, `.class` for a style and `key="value"` for options.

````markdown {corpus="page/attributes" live=false}
## Rollback {#rollback .wide}

::: info {#note-1}
Attributes go on the first line of a block.
:::

```chart {height=240}
title: Requests
type: bar
labels: [Mon, Tue]
values: [3, 5]
```

![Gateway request path](assets/arch.png){width=50% align=right}
````

## Block IDs {#block-ids}

When you comment on, cite or link to a block, the editor adds an
8-character block ID to it on save. Links, tasks, comments and AI
citations then point at that exact block, even after the text around it
changes.

- Review the diff ^blk-7f3akq2m

````markdown {corpus="inline/block-id"}
- Review the diff ^blk-7f3akq2m
````

## In this part

- [Text and links](text.md): emphasis, links, wikilinks, mentions, dates and issue keys.
- [Structure, lists and tables](structure.md): headings, lists, tasks, quotes, code, math and tables.
- [Callouts and containers](containers.md): callouts, toggles, columns, tabs and nesting.
- [Numbers and facts](numbers.md): stats, facts, progress, steps and cards.
- [Charts and data tables](charts.md): eight chart types and sortable tables.
- [Plans, timelines and risk](plans.md): timelines, roadmaps, risk matrices and boards.
- [Images, links and embeds](media.md): images, link cards, embeds and page grids.
- [Interactive blocks](interactive.md): calculators and live collection views.
- [Where things render](rendering.md): how each block looks on GitHub, in PDF and in Word.
