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
- Decisions: D85, D86, D87
- Issue: #165, #166, #167, #168, #169, #170
- Pull requests: #210
- Crate: `vellum-mcp` (layer `edge`)

:::

## Context

The brief (v2.0, section 10) makes MCP the AI door into Vellum: a server
on the current protocol revision, Streamable HTTP at `/mcp` behind
OAuth 2.1, stdio through `vellum mcp`, a fixed set of tools, resources
and prompts, drafts for model writes, scopes, audit, per-token rate
limits, an injection posture, `llms.txt`, `?format=md`, a Markdown
sitemap and client setup snippets with "Test connection".

We checked every protocol detail against the official specification and
schema for revision **2026-07-28** before building. The revision is
stateless: there is no `initialize` handshake and no session; every
request carries its protocol version and client capabilities in `_meta`;
`server/discover` replaces the handshake; `subscriptions/listen` replaces
`resources/subscribe` and the standalone GET stream. Where the spec and
the brief differ, the decisions below say which way we went.

## Options considered

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
- Everything a model reads passes the injection guard: invisible and
  bidirectional control characters are stripped, Markdown comments are
  removed, and blocks that read like instructions to the model are
  flagged in `_meta["dev.vellum/warnings"]` with their block ids. Server
  instructions tell the model that workspace content is data, not
  instructions.
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
- Discrepancies with the brief, resolved toward the spec:
  - `logging/*`, `roots/*` and `sampling/*` are deprecated in 2026-07-28
    and are not implemented; tracing uses `traceparent` and `tracestate`
    in `_meta` instead.
  - Resource subscriptions use `subscriptions/listen` only. Legacy
    `resources/subscribe` is not offered.
  - Lists and reads carry the revision's cache hints (`ttlMs`,
    `cacheScope`): 60 s for lists, 30 s private for reads, one hour for
    discovery.
  - Dynamic client registration is deprecated; clients identify with
    Client ID Metadata Documents at the authorization server.
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
- Streamable HTTP is POST only (GET and DELETE answer 405), validates
  `Origin`, `MCP-Protocol-Version`, `Mcp-Method` and `Mcp-Name`, and
  streams `subscriptions/listen` over SSE with keep-alive comments;
  closing the stream cancels the subscription.

## Consequences

- Clients on either protocol era work today, and dropping 2025-11-25
  later is a deletion, not a redesign.
- Permission filtering lives behind the `Workspace` port, so search,
  resources, prompts, `llms.txt` and `?format=md` can only show what the
  caller may read.
- `vellum mcp` without `--demo` fails with a clear message until the
  workspace services are wired into the binary; that wiring belongs to
  the app crate.

## History

- 2026-10-29: accepted in #210.
