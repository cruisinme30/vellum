---
id: 01M63CFE6128Z1BQPN30Q5MK2F
title: AI and MCP
vellum: 1
---

# AI and MCP

Vellum's MCP server is part of the binary. It is reachable at `/mcp`
over HTTPS with OAuth sign-in, and as `vellum mcp` over standard input
and output for local clients.

## Admin controls {#controls}

In **Admin → AI**:

::: facts

- MCP server: on or off for the whole workspace
- Allowed scopes: the most any client may ask for; `write:direct` and `admin` are off by default
- Rate limits: calls per minute per token
- Clients: every connected client, who connected it and when it was last used, with **Revoke**
- Models for built-in features: the provider and key used for summarise, ask and draft, or none

:::

## Safety {#safety}

Every call is checked against the user's permissions and recorded in
the audit log with its arguments. Model writes are drafts unless the
user granted `write:direct`. Content is treated as data: invisible text
is stripped and instruction-like blocks are flagged before a model sees
them.

## Search for models {#search}

MCP search, and search in the app, combine keyword search with optional
local embeddings, computed on your server by a small ONNX model. Nothing
is sent to an external vector service, and nothing is downloaded.

To turn embeddings on, put a sentence-embedding model exported to ONNX
in a folder the server can read (for example `all-MiniLM-L6-v2` or
`bge-small-en-v1.5`, with its `model.onnx`, `vocab.txt` and
`config.json`) and set:

::: facts

- `VELLUM_EMBEDDINGS`: the model folder; unset, search is keyword-only
- `VELLUM_EMBEDDINGS_IDLE_MINUTES`: how long the model stays in memory unused, 15 by default

:::

A folder Vellum cannot use stops the server at start with the reason.
Pages indexed before you turned embeddings on get them in the
background; the server log shows the progress. On Postgres, run
`vellum reindex` after switching to a different model.

## Markdown for models and crawlers {#llms-txt}

Every Vellum server answers `/llms.txt` (the format at `llmstxt.org`)
format) and `/sitemap.md` with the pages the caller may read, and any
page URL with `?format=md` returns the page as Markdown. A signed-in
browser or a personal API token sees its own pages; an anonymous
visitor sees none.
