---
id: 01M63BKA58RWSRMRSKF96BQ1B6
title: "ADR 0020: Gateway, service registration and identity"
status: verified
owner: "@cruisinme30"
tags: [adr, api, identity, security]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D88, D89
- Issue: #84, #85, #148, #46
- Pull requests: #39, #234

:::

## Context

The brief runs eleven services in one binary, each able to run alone, and
exposes them through one gateway with REST (OpenAPI 3.1) and `GraphQL`
with full mutation coverage. The services are built in parallel by
separate workstreams, so they need one stable way to plug into the
gateway before any of them is written. The crate layering rule (D33) puts
the gateway (`edge`) above the services (`service`), so the seam cannot
live in the gateway crate.

## Decisions

### D88: services register typed operations in `vellum-contracts`

- A service implements `vellum_contracts::Service`: a name, a list of
  `Operation`s and a readiness probe. An operation is one typed function
  (`serde` + `schemars` input and output) with an ID
  (`<service>.<name>`), a REST method and path below `/api/v1/`, the
  token scope it needs, an optional re-authentication window and a time
  budget.
- The gateway derives, from that single declaration, the REST route, the
  OpenAPI 3.1 operation and a `GraphQL` query or mutation. REST and
  `GraphQL` therefore cover exactly the same mutations, by construction,
  and the generated TypeScript client covers both.
- The gateway owns every cross-cutting concern: authentication into a
  `Principal`, scope and freshness checks, `Idempotency-Key` handling,
  `X-Request-Id`, the timeout from the budget, one circuit breaker per
  service, `Server-Timing`, security headers, CSRF, `/healthz` and
  `/readyz`. Services keep per-object authorization (deny by default,
  through the authorization service).
- Surfaces that are not request and response (collaboration `WebSocket` rooms,
  streamed downloads, MCP Streamable HTTP) export their own router; the
  binary mounts it on the gateway, which applies the same middleware and
  puts the `Principal` in the request.
- Start-up refuses a malformed operation: a bad ID, a path outside
  `/v1/`, a `GET` mutation, or a path parameter missing from the input.

| Option | One source for REST and `GraphQL` | Services free of HTTP crates | Typed contract tests | Chosen |
| --- | --- | --- | --- | --- |
| Typed operation registry | ✓ | ✓ | ✓ | ✓ |
| Each service ships an `axum` router and a `GraphQL` object | ✗ | ✗ | ◐ | ✗ |
| Hand-written gateway handlers per service | ◐ | ✓ | ◐ | ✗ |

### D89: the gateway owns the edge, including the public-pages origin

- TLS is 1.3 only (`rustls` with `aws-lc-rs`), from PEM files or from
  ACME (TLS-ALPN-01) with the certificate cache in the data directory.
  HTTP/1.1 and HTTP/2 share one listener; shutdown drains connections
  and turns `/readyz` to 503 first.
- Every response carries a strict CSP, `nosniff`, frame denial,
  COOP/CORP and, over HTTPS, HSTS.
- CSRF: cross-site `Sec-Fetch-Site` and foreign `Origin` are refused,
  and cookie sessions also send `X-CSRF-Token`, an HMAC of the session
  binding. Bearer tokens are exempt.
- `Idempotency-Key` makes writes safe to retry: the first response is
  stored per caller for 24 hours and replayed; a different request
  under the same key is a 422, a concurrent one a 409.
- Rate limits reuse `vellum-security`'s GCRA limiter per caller.
- Share links (`/s/{token}`) and published pages (`/p/{path}`) are served
  only on a separate public origin, with a sandboxing CSP, no scripts and
  no app cookies. A `PublicSite` port resolves them, so the gateway
  never reads share records itself.

## Consequences

- A new endpoint is one `Operation` in the owning service; nothing in the
  gateway changes.
- `GraphQL` arguments and results follow the JSON Schema of the Rust
  types, so `GraphQL` gains no feature REST lacks; bespoke `GraphQL`
  shapes are out of scope by design.
- ID types need `JsonSchema` implementations to appear in inputs; the
  kernel provides them.

## History

- 2026-10-29: D88 accepted in #39.
- 2026-10-29: D89 accepted in #234.
