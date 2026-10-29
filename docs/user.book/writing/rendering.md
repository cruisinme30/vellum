---
id: 01M63CFE695JHR5CT1S8HH3WDC
title: Where things render
vellum: 1
---

# Where things render

Vellum Markdown is designed to degrade well. Standard Markdown is rich
everywhere; Vellum blocks are rich in Vellum and readable as plain text
anywhere else.

| Block | In Vellum | On GitHub | In PDF and Word |
| --- | --- | --- | --- |
| Headings, lists, tables, code, images | rich | rich | rich |
| Containers, stats, cards, steps | rich | readable text | styled |
| Chart, roadmap, timeline, risk matrix, architecture | SVG | the fence as code | vector image |
| Mermaid | themed SVG | SVG, unthemed | vector image |
| D2, PlantUML, Graphviz, Excalidraw, draw.io | SVG from Kroki | the fence as code | vector image |
| Collection view, calculator, issue chips | live | code or text | current state as a table or text |
| Embeds | sandboxed player | the link | the link with a preview image |

## Exports

- **PDF** has two profiles: *print* (Typst; page numbers, running heads, a table of contents) and *screen* (looks exactly like the page).
- **Word** keeps headings, lists, tables and comments; charts and diagrams become vector images.
- **Markdown** is the file itself, with assets in a zip.
- **HTML** is a single self-contained file.

## Plain Markdown access

Add `?format=md` to any page URL to get its Vellum Markdown, subject to
the same permissions as the page.
