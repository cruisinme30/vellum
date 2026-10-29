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
