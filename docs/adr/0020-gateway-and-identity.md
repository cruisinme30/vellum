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
- Decisions: D88
- Issue: none (recorded before the issue rule, D119)
- Pull requests: #39

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

## Alternatives

- **Each service ships an `axum` router and a `GraphQL` object**: REST
  and `GraphQL` drift apart, and services pull in HTTP crates.
- **Hand-written gateway handlers per service**: the gateway changes for
  every endpoint and coverage depends on discipline.

## Consequences

- A new endpoint is one `Operation` in the owning service; nothing in the
  gateway changes.
- `GraphQL` arguments and results follow the JSON Schema of the Rust
  types, so `GraphQL` gains no feature REST lacks; bespoke `GraphQL`
  shapes are out of scope by design.
- ID types need `JsonSchema` implementations to appear in inputs; the
  kernel provides them.
