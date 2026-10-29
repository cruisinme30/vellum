---
id: 01M63CFE699A568XC5KR9935JE
title: Diagram errors
vellum: 1
---

# Diagram errors

When a diagram's source does not parse, Vellum does not hide it or break
the page. It shows the source as code with a red bar that names the line
and the problem, and it keeps the last good drawing in exports until you
fix it.

```mermaid
flowchart LR
  A[Client] --> B{Gateway
```

`````markdown {corpus="mermaid/error"}
```mermaid
flowchart LR
  A[Client] --> B{Gateway
```
`````

The example above is missing the closing `}`, so Vellum reports
"line 2: unclosed shape `{`".

## Common mistakes

| Symptom | Cause | Fix |
| --- | --- | --- |
| Unclosed shape | A `[`, `(` or `{` without its pair | Close the bracket |
| Unknown diagram type | A typo in the first line, such as `flowchar` | Use a type from [the list](mermaid.md) |
| Arrow not recognised | `->` in a flowchart | Flowcharts use `-->` |
| Label cut short | A quote or a pipe character inside a label | Wrap the label in quotes: `A["Say \"hi\""]` |
| Diagram shows as code with "Kroki unavailable" | Your admin has not enabled Kroki, or it is down | Ask your admin; Mermaid and architecture diagrams never need Kroki |

## When Kroki is down

D2, PlantUML, Graphviz, Excalidraw and draw.io diagrams are drawn by the
Kroki service. If it is down, Vellum shows the last drawing it made with
a small notice, plus the source. Nothing is lost, and the drawing
refreshes once Kroki is back.
