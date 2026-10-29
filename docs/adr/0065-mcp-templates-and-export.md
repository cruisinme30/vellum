---
id: 01M64V94164W7V117G8H3HMNWD
title: "ADR 0065: MCP templates and export"
status: draft
owner: "@cruisinme30"
tags: [adr, mcp, ai]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: proposed
- Decisions: D264, D265
- Issue: #513
- Pull requests: the integration batch with feat/mcp-templates-export

:::

## Context

With MCP inside `vellum serve` (ADR 0019, D217–D219), two tools still
said "not available": page templates, which the `write-prd` prompt and
drafting from a template need, and `export`. Vellum has no template
store, and export routes are being added to the server by the export and
import work.

## Decisions

### D264: page templates are pages in a Templates folder

- A template is any page in a folder named `Templates` in a space the
  user can open; there is no separate store, so people edit templates
  like any page and permissions are the drive's.
- Its name is its title in lowercase with words joined by hyphens
  (`PRD` is `prd`); when two share a name, the first by title wins.
- A read-only `get_template` tool lists templates without a name, and
  returns one's Markdown, front matter removed, with a name. The
  `write-prd` prompt reads `prd` the same way.
- The tool list stays fixed for every caller (D85); it grows by one.

### D265: MCP export uses the server's export routes

- The `export` tool calls the export service's own operation as the
  user, like the other services (D218), and returns its short-lived
  download link, file name, MIME type and size.
- No second export pipeline lives in the MCP crate or the adapter.

## Alternatives

- **A template store with its own API**: more to build and secure, and
  templates would not be ordinary pages. Rejected (D264).
- **Export inside the MCP adapter**: a second copy of rendering and
  limits. Rejected (D265).

## Consequences

- Any team can add a template by adding a page to a Templates folder.
- Export over MCP depends on the export routes landing first.
