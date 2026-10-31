---
id: 01M63CFE699P22GB36Z2F8NN82
title: Diagrams
vellum: 1
---

# Diagrams

Vellum draws diagrams from text, so they live in the page, diff cleanly
in version history and never go stale as screenshots.

::: cards

- 🧜 **Mermaid** Every Mermaid diagram type, rendered on the server and themed to
  the page.
- 🏛 **Architecture** Lanes and arrows for system overviews, with no layout to
  fight.
- 🧰 **Kroki** D2, PlantUML, Graphviz, Excalidraw and draw.io, when your admin
  enables the Kroki service.

:::

## How diagrams render

Vellum renders diagrams on the server when you save, and caches the SVG
with the page. Readers get the finished SVG, so a page full of diagrams
opens as fast as a page of text, and exports to PDF and Word embed the
same vector image.

Diagrams follow the page theme: colours, fonts and dark mode all match.
Long labels wrap instead of overflowing their boxes.

## Exporting a diagram

Select a diagram and choose **Download SVG** or **Download PNG** from its
block menu. **Copy source** copies the text.

## In this part

- [Mermaid diagrams](mermaid.md) shows every Mermaid type.
- [Architecture and Kroki diagrams](kroki.md) covers the `architecture` fence
  and the Kroki types.
- [Diagram errors](errors.md) explains what you see when a diagram does not
  parse.
