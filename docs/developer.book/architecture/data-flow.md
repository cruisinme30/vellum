---
id: 01M63CFE64R1N91EY461P6BRTF
title: Read and write paths
vellum: 1
---

# Read and write paths

## Reading a page {#read}

Pages are rendered when they are written, not when they are read. A read
is an authorization check and a cache lookup.

```mermaid
sequenceDiagram
  participant B as Browser
  participant G as Gateway
  participant A as Authorization
  participant C as Render cache
  B->>G: GET /d/roadmap (If-None-Match)
  G->>A: may this user view this page?
  A-->>G: yes (one indexed lookup)
  G->>C: rendered HTML for version 42
  alt ETag matches
    G-->>B: 304 Not Modified
  else
    C-->>G: HTML
    G-->>B: 200 with ETag and Server-Timing
  end
```

The cached read target is p95 under 50 ms.

## Writing a page {#write}

```mermaid
sequenceDiagram
  participant E as Editor
  participant R as Collab room
  participant P as Postgres
  participant W as Content service
  participant S as Blob store
  E->>R: Yjs update
  R->>R: batch for up to 50 ms
  R->>P: append updates
  R-->>E: broadcast to other editors
  Note over R,W: after 30 s idle or Done
  R->>W: snapshot as Vellum Markdown
  W->>S: write file (new version)
  W->>W: render, index, notify
```

Updates are durable once appended to Postgres; the Markdown file is
written after 30 seconds of quiet. Writes carry request IDs, so a retried
write is applied once.

## External edits {#external}

The watcher notices a file changed outside Vellum, finds the page by the
`id` in its front matter, and merges three ways against the last version
Vellum wrote: diff-match-patch for text and diff3 for manifests. When
the same lines changed on both sides, the page keeps Vellum's text and
the outside version is saved as a conflict copy. The last write never
silently wins.

## Live collaboration {#collab}

Each open document is a room that holds the `Yjs` tree. Editors connect
over one WebSocket per document at `/collab/{space_id}/{doc_id}`, using
the stock `Yjs` sync and awareness protocol.

```mermaid
sequenceDiagram
  participant A as Priya's editor
  participant B as Dev's editor
  participant R as Room
  participant L as Update log
  participant W as Content service
  A->>R: join (sync step 1 and 2)
  B->>R: join
  A->>R: update: types a sentence
  R-->>B: fan out at once
  R->>L: append, batched up to 50 ms
  B->>R: awareness: cursor moved
  R-->>A: who holds each cursor (message 102)
  Note over R,W: 30 s idle, Done, or the last editor leaves
  R->>W: write a Markdown version, credit both editors
```

::: tip
With several replicas, the cluster router sends every connection for a
document to the one replica that owns its room, so a room never runs
twice.
:::

## Offline sync {#sync}

The web and desktop apps save every change on the device first and push
it later. The sync service keeps one change log row per record, so a
device that was offline for a month downloads each changed record once.

```mermaid
sequenceDiagram
  participant D as Device store
  participant S as Sync service
  participant M as Owning service
  participant L as Change log
  D->>S: push mutations 41, 42, 43
  S->>S: skip IDs at or below the last processed one
  S->>M: run each mutator in order
  M-->>S: changed records
  S->>L: record each change at the next sequence number
  D->>S: pull since cursor 9000
  S->>L: rows after 9000 the person may read
  L-->>D: up to 1,000 rows, and more: true
```

## Export {#export}

Every export lowers the page into one intermediate flow first, with live
blocks frozen to their current state. Each backend reads only that
flow.

```mermaid
flowchart LR
  P[Page or book] --> F[Lowering: headings, tables, figures, cross-references]
  F --> T[Typst, in-process: PDF]
  F --> O[OOXML writer: Word]
  O --> G[Google Docs: Word uploaded with conversion]
  F --> H[HTML and static sites]
  F --> M[Markdown and mdBook]
  R[Render: charts and diagrams as SVG] --> F
```
