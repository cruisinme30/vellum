---
id: 01M63CM8RWW55HMFPG8FDAHQ7Y
title: "ADR 0019: MCP server"
status: verified
owner: "@cruisinme30"
tags: [adr, mcp, ai, security]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D85, D86, D87, D217, D218, D219
- Issue: #165, #166, #167, #168, #169, #170, #171, #316, #88, #440, #511
- Pull requests: #210, and the integration batch with feat/mcp-in-serve
- Crate: `vellum-mcp` (layer `edge`)

:::

## Context

The brief (v2.0, section 10) makes MCP the AI door into Vellum: a server
on the current protocol revision, Streamable HTTP at `/mcp` behind
OAuth 2.1, stdio through `vellum mcp`, a fixed set of tools, resources
and prompts, drafts for model writes, scopes, audit, per-token rate
limits, an injection posture, `llms.txt`, `?format=md`, a Markdown
sitemap and client setup snippets with "Test connection".

We checked every protocol detail against the specification for revision
**2026-07-28**, which is stateless (no handshake, no session; version and
capabilities ride in `_meta`). Where it and the brief differ, the
decisions say which way we went. D217–D219 put the server inside
`vellum serve` over the real workspace, on several replicas.

## Decisions

### D85: scopes, drafts and a fixed tool list

- Scopes are `read`, `write:draft`, `write:direct`, `work` and `admin`,
  with `admin` implying all, `write:direct` implying `write:draft`, and
  both `write:draft` and `work` implying `read`. This matches the gateway
  seam, so one token means the same thing everywhere.
- `tools/list` is the same for every caller and never changes
  (`listChanged: false`). Each tool names its scope in
  `_meta["dev.vellum/scope"]`; a call without it is answered with
  HTTP 403 and a `WWW-Authenticate: Bearer error="insufficient_scope"`
  challenge, so clients can step up. A list that shrank or grew with the
  token would make client caches wrong and leak nothing useful.
- Model writes to pages, blocks and rows land as **drafts** attributed to
  the user and the client. A direct write needs both `direct: true` in the
  arguments and the `write:direct` scope. Issues a model files land in
  Triage. Comments, tasks and issue updates are written directly under
  `work`, because they are already reviewable in place.
- Everything a model reads passes the injection guard (hidden and
  bidirectional characters and comments stripped, instruction-like blocks
  flagged in `_meta["dev.vellum/warnings"]`), and every string of every
  tool result passes it once more on the way out (#316).
- Every call is audited (user, client, token, tool, URIs touched,
  outcome, request id and trace id) and rate limited per token with the
  shared GCRA limiter from `vellum-security` (burst 30, 120 per minute by
  default). A limited call gets HTTP 429 with `Retry-After`.

### D86: dual-era protocol support

- The server speaks **2026-07-28** and also accepts **2025-11-25**
  clients, because most clients in the field still send `initialize`.
  A legacy `initialize` is answered without minting a session; on HTTP
  the version then comes from the `MCP-Protocol-Version` header, and on
  stdio it is remembered for the connection.
- Toward the spec where the brief differs: no `logging/*`, `roots/*` or
  `sampling/*` (tracing rides in `_meta`); `subscriptions/listen` only;
  cache hints of 60 s for lists, 30 s private for reads, an hour for
  discovery; Client ID Metadata Documents, not dynamic registration.
- Vellum's own error codes sit outside the JSON-RPC and MCP reserved
  ranges: -31001 unauthenticated, -31003 insufficient scope, -31029 rate
  limited. Protocol errors use the spec's codes (-32020 header mismatch,
  -32021 missing client capability, -32022 unsupported version, -32602
  resource not found).
- Subscriptions honour only URIs the caller can read when they listen,
  at most 100 per stream; `resources` reports `listChanged: true`.

### D87: ports, adapters and the auth split

- The identity service owns the OAuth 2.1 authorization server (PKCE,
  RFC 8414 metadata, `iss`, CIMD). The MCP server is only a **resource
  server**: it publishes RFC 9728 protected-resource metadata, checks the
  RFC 8707 audience (`<base>/mcp`) and expiry, and verifies tokens
  through the `TokenVerifier` port.
- The crate reaches the rest of Vellum only through the `Workspace`,
  `AuditSink` and `TokenVerifier` ports. The app wires real services
  behind them and mounts the router raw through `Gateway::mount`. An
  in-memory adapter seeded with the Northstar workspace backs the tests
  and `vellum mcp --demo`.
- Streamable HTTP is POST only, checks `Origin` and the `MCP-*` headers,
  and streams `subscriptions/listen` over SSE with keep-alives.

### D217: `/mcp` inside `vellum serve`, tokens from identity

- With an index configured, serve mounts `/mcp` (CSRF-exempt, bearer
  only) and merges the RFC 9728 documents into `/.well-known`.
- Identity's `verify_token` backs `TokenVerifier`. OAuth tokens must name
  `<base>/mcp` as audience; personal API tokens carry none and only work
  on the server that issued them, so they count as issued for `/mcp`.
- A bad bearer on `/mcp` reaches the endpoint as anonymous, which answers
  with the `WWW-Authenticate` challenge clients follow to sign in again.
- Calls go to the admin audit log as `mcp.<method>`, actor `model`.

### D218: the real workspace behind the port; drafts are pages

- The adapter calls the services' own operations (search, Work, My Work,
  collections, books) as the token's user, and the drive as
  `Actor::Model` for that user, so permissions and activity are theirs.
- A draft never changes an existing page: a new page gets
  `status: draft`; an edit or a set of rows becomes a draft page in the
  user's My Drive with `draft_of` and `base_version`. Model pages carry
  `authored_by`, `on_behalf_of` and `client` in front matter.

### D219: one cluster for limits and listeners

- The per-token limit also counts in the replicas' shared table
  (`mcp:<token>`, D173), so spreading calls over replicas does not help.
- `subscriptions/listen` is fed from the cluster change feed, which every
  replica follows, its own writes included: a page saved on one replica
  reaches listeners on all of them within the feed's poll (250 ms).

## Alternatives

- **Target 2025-11-25 only**: what most clients send today, but it needs
  sessions and a handshake the brief's revision removed. Rejected; we
  accept it as a fallback instead (D86).
- **Target 2026-07-28 only**: matches the brief, but would turn away most
  clients in the field for now. Rejected for the same reason.
- **Filter `tools/list` by scope**: hides tools a token cannot call, but
  makes the list differ per token and breaks client caches. Rejected in
  favour of a fixed list and call-time checks (D85).
- **Run our own authorization server inside the MCP crate**: duplicates
  identity. Rejected; MCP is a resource server only (D87).
- **An MCP-only path to the data**: rejected; one set of checks (D218).

## Consequences

- Clients on either protocol era work today, and dropping 2025-11-25
  later is a deletion, not a redesign.
- Permission filtering lives behind the `Workspace` port, so search,
  resources, prompts, `llms.txt` and `?format=md` can only show what the
  caller may read.
- MCP clients connect to `<server>/mcp`; `vellum mcp` stays for the demo
  and private spaces. Export and page templates over MCP are not served
  yet; those tools say so.
