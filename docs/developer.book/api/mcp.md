---
id: 01M63CFE6534FYF57RQC9JDD77
title: The MCP server
vellum: 1
---

# The MCP server

Vellum implements the Model Context Protocol, revision 2026-07-28, in
the stateless style: each request carries the protocol version and the
client's capabilities in `_meta`, and there is no session to keep.

::: facts

- Transports: Streamable HTTP at `/mcp`, and standard input and output with `vellum mcp`
- Sign-in: OAuth 2.1 with Client ID Metadata Documents
- Discovery: `server/discover`
- Change feed: `subscriptions/listen`
- Tracing: `traceparent` in `_meta` joins the client's trace

:::

`tools/list` is deterministic, and list and read results carry cache
hints. The deprecated roots, sampling and HTTP+SSE transport are not
implemented.

## Tools {#tools}

`search`, `get_doc`, `get_blocks`, `get_outline`, `create_draft`, `propose_edit`, `create_issue`, `update_issue`, `add_comment`, `create_task`, `query_collection`, `upsert_rows`, `export` and `list_changes`.

## Resources {#resources}

| URI | Is |
| --- | --- |
| `vellum://doc/{id}` | A page as Vellum Markdown |
| `vellum://book/{id}` | A book's outline and sections |
| `vellum://issue/KEY-n` | An issue |
| `vellum://collection/{id}?view=` | A collection view as rows |
| `vellum://space/{slug}/tree` | A space's page tree |

## Prompts {#prompts}

`summarise-space`, `write-prd`, `weekly-status`, `verify-page` and `triage-issue`.

## Retrieval {#retrieval}

The retrieval unit is a block and the context unit is the heading
section around it. Search fuses BM25 keyword results from tantivy with
optional local ONNX embeddings by reciprocal rank, filtered by the
caller's permissions before ranking.

## Writes {#writes}

`create_draft` and `propose_edit` create drafts recorded as
`authored_by: model` and `on_behalf_of: <user>`. Only a token with
`write:direct` can change a page without review.
