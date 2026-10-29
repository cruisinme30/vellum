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

MCP search combines keyword search with optional local embeddings,
computed on your server by a small ONNX model. Nothing is sent to an
external vector service. Turn embeddings on in **Admin → AI → Semantic
search**; Vellum builds them in the background.
