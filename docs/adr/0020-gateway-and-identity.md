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
- Decisions: D88, D89, D90
- Issue: #84, #85, #148, #46, #86, #88
- Pull requests: #39, #234, #347

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

### D89: the gateway owns the edge, including the public-pages origin

- TLS 1.3 only (`rustls` with `aws-lc-rs`), from PEM files or ACME
  (TLS-ALPN-01); HTTP/1.1 and HTTP/2 share one listener, and shutdown
  turns `/readyz` to 503 before draining.
- Every response carries a strict CSP, `nosniff`, frame denial,
  COOP/CORP and, over HTTPS, HSTS.
- CSRF: cross-site `Sec-Fetch-Site` and foreign `Origin` are refused;
  cookie sessions also send `X-CSRF-Token`, an HMAC of the session
  binding. Bearer tokens are exempt.
- `Idempotency-Key` stores the first response per caller for 24 hours
  and replays it; a different request under the key is a 422, a
  concurrent one a 409.
- Rate limits reuse the `vellum-security` GCRA limiter per caller.
- Share links (`/s/{token}`) and published pages (`/p/{path}`) are served
  only on a separate public origin with a sandboxing CSP and no scripts,
  through a `PublicSite` port.

### D90: identity issues opaque, hashed credentials and is the `OAuth` server

- Every credential (session cookie, `vlm_` API token, `OAuth` code,
  access and refresh token, break-glass code) is 32 random bytes behind a
  type prefix, stored only as its SHA-256. Passwords use Argon2id.
- Sessions are an `HttpOnly`, `SameSite=Lax` cookie
  (`__Host-vellum_session` over HTTPS) with 14-day idle and 30-day
  absolute expiry; the session hash is the CSRF binding.
- Sign-in locks out an address after 5 failures and an IP after 20, with
  doubling waits up to 15 minutes. Admin operations need a sign-in within
  15 minutes; tokens never satisfy that for long.
- The `OAuth` 2.1 server takes public clients only: PKCE `S256`, Client ID
  Metadata Documents fetched through the SSRF guard, the built-in
  `vellum-desktop` client with any-port loopback redirects, RFC 8707
  audiences (`<base>/mcp` for MCP) and rotating refresh tokens whose
  reuse revokes the grant.
- Break-glass: `vellum admin break-glass` writes a one-time code to the
  data directory; redeeming it opens a 15-minute administrator session.
- Records live in `.vellum/identity.json` (mode 0600) until the SQL index
  lands. SSO (OIDC, SAML) follows in its own change.

## Alternatives

- **Each service ships an `axum` router and a `GraphQL` object**: REST
  and `GraphQL` drift apart, and services pull in HTTP crates.
- **Hand-written gateway handlers per service**: the gateway changes for
  every endpoint and coverage depends on discipline.
- **Signed JWT access tokens**: cannot be revoked before expiry, and
  each resource server would need key distribution; lookups are cheap.
- **Share pages on the app origin**: user content would run beside the
  signed-in session's cookies.

## Consequences

- A new endpoint is one `Operation` in the owning service; nothing in the
  gateway changes.
- `GraphQL` arguments and results follow the JSON Schema of the Rust
  types, so `GraphQL` gains no feature REST lacks; bespoke `GraphQL`
  shapes are out of scope by design.
- ID types need `JsonSchema` implementations to appear in inputs; the
  kernel provides them.
