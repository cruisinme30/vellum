---
id: 01M65DV1A3SBTPQ6937JJHQ0ZP
title: "ADR 0061: Degraded modes people can see, and the page render cache"
status: verified
owner: "@cruisinme30"
tags: [adr, reliability, performance, render]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D267, D268, D269, D270
- Issue: #191, #74
- Pull requests: to be linked when merged

:::

## Context

The brief asks for named degraded modes with tests (blob store outage,
Redis loss, Kroki down, collab re-home) and for cached page reads under
50 ms at p95 with `304` answers
([#191](https://github.com/cruisinme30/vellum/issues/191),
[#74](https://github.com/cruisinme30/vellum/issues/74)). ADR 0044 (D188)
already keeps servers in rotation when the bucket is down, answering file
reads and writes with `503` and `Retry-After`. The renderer already shows
the last good diagram, or its source, when Kroki is down, and a drain
saves collab rooms. Vellum has no Redis: shared state is in Postgres (ADR
0040). What was missing: people could not tell, saves failed outright,
re-homing under load had no test, and every page read rendered again.

## Decisions

### D267: the web app says when part of Vellum is down

- The HTTP client reports every answer to one health monitor: a `503`
  turns it degraded, the next answer below 500 turns it back.
- The shell shows a status banner while degraded: reading goes on, saving
  may fail, and Vellum retries on its own.
- A `503` is retried once after its `Retry-After` when that is at most
  5 s. The idempotency key every write carries makes the retry safe.
- Network failures stay the browser's offline state, not this banner.

### D268: collab re-home is tested under typing

- Three editors type while their node drains. Edits sent before the close
  are in the log; later ones stay in each editor's document.
- They rejoin another node with those documents, and every line from
  every editor reaches them and a new reader.
- The weekly chaos run kills a node and cuts the bucket under k6 load,
  including the collab scenario (ADR 0022, D255).

### D269: rendered pages are cached by what they were rendered from

- The key is a SHA-256 of the page ID, `RENDER_VERSION` and the Markdown.
  A save makes a new key, so nothing is ever invalidated.
- Each replica keeps up to 2048 pages and 64 MiB of HTML in an LRU, none
  over 512 KiB.
- Pages that depend on the reader (live `::: issues` blocks) or that show
  a stale diagram are rendered every time.
- The drive still checks permission and reads the file on every request.
- `GET /api/v1/pages/{id}/render` has a 50 ms p95 budget by default, so
  `vellum budgets` and the console mark it when it is slower.

### D270: queries opt in to `ETag` and `304 Not Modified`

- `Operation::revalidate()` makes the gateway send a weak `ETag` over the
  JSON with `Cache-Control: private, no-cache`.
- A matching `If-None-Match` gets `304` with no body, after the caller's
  authentication and permission checks.
- Only the page render opts in. Other answers keep no cache headers, so
  nothing sensitive lands in a browser cache by default.

## Alternatives

- **A polling status endpoint**: every open tab would poll the probes;
  reacting to answers the app already gets costs nothing.
- **Render on write into the bucket**: renders pages nobody reads, and a
  bucket read is slower than memory; the content key makes a cold
  replica's first read the only render.
- **One cache shared by all replicas**: adds a service and a network hop
  for something every replica can rebuild in one render.
- **An `ETag` on every GET**: would let browsers keep answers such as token
  lists; opt-in keeps the default conservative.

## Consequences

- A resolver method that answers per reader must mark the page personal,
  or one reader's view would be cached for everyone.
- Raising `RENDER_VERSION` empties the cache in effect, by changing keys.
- Other read-heavy queries can opt in to `304` with one call.
