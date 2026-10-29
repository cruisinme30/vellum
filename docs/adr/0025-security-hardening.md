---
id: 01M63CH6X3NH097K3AW5NMKCHW
title: "ADR 0025: Outbound fetches, uploads and E2E private spaces"
status: verified
owner: "@cruisinme30"
tags: [adr, security, foundation]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D103, D104, D105

:::

## Context

Brief §11 and R-SE-5 to R-SE-10 list defences that cut across services:
outbound fetches for link previews, imports, webhooks and Kroki; file
uploads; secret scanning on save; rate limits and lockout; and optional
end-to-end encrypted private spaces. Each service could implement these
on its own, but the dangerous details, such as `DNS rebinding`,
decompression bombs and small-order `X25519` keys, are easy to get wrong
more than once. `vellum-security`, a foundation crate, holds them in
one place, and services call it.

## Decisions

### D103: every outbound fetch goes through `SafeClient`

- No service builds its own HTTP client for user-influenced URLs.
  `vellum_security::ssrf::SafeClient` is the only one.
- Only `https` is allowed (plus `http` when the policy allows it), on
  ports 80 and 443, with no credentials in the URL. Single-label names
  and `localhost`, `.local`, `.internal`, `.home.arpa`, `.lan` and `.corp`
  are refused.
- Every resolved address must be public `unicast`. The check covers `IPv4`
  special ranges, `IPv6` global-only, and `IPv4` embedded in mapped, `NAT64` and
  `6to4` forms. The check runs inside the resolver, so the address that is
  checked is the address that is connected to, which defeats DNS
  rebinding.
- Each redirect hop is checked again, up to 5 hops. Proxies are ignored,
  and the timeout and body size are capped.
- Operators can trust exact host names (for example an internal Kroki) in
  configuration. Wildcards and IP ranges are not supported, so a typo
  cannot open the private network.

### D104: uploads are identified by content, re-encoded and scanned

- The type comes from magic bytes, never from the client's
  `Content-Type`. The extension must agree with it, and unknown types are
  refused.
- Raster images are decoded under limits of 16384 px per side and a
  512 MiB allocation cap, then re-encoded. This strips `EXIF`, comments and
  polyglot payloads.
- SVG, ZIP, Office files and text are always served with
  `Content-Disposition: attachment`. SVG is inlined only after the
  renderer's sanitizer.
- The `ClamAV` hook (`clamd` `INSTREAM`) is optional. When it is configured
  it fails closed: if the scanner is unreachable, the upload is refused.
- Secret scanning on save warns by default, and an admin policy can block
  instead. Findings carry a position and a redacted preview, never the
  secret. Lines marked `vellum:allow-secret` are skipped.
- Rate limits use `GCRA` with a bounded key table, and lockout uses
  exponential back-off. The Org profile runs the same algorithm
  against `Redis`, with the in-process limiter as fallback.

### D105: E2E spaces hold public keys only and refuse plaintext features

- A space marked E2E stores only recipient envelopes
  (`vellum_crypto::recipients`, `vellum_sealed_store::Mode::EndToEnd`).
  Each document is sealed on a member's device to every member device's
  `X25519` key.
- The server's `KeyRegistry` stores public keys per space, member and
  device. It refuses small-order keys, duplicates, more than 255 keys and
  removal of the last key.
- Removing a device or member returns the keys that clients must strip
  from every envelope. This protects future writes only: a removed member
  who kept old ciphertext and their key can still read it. The UI says so
  when a member is removed.
- In an E2E space the server refuses search, render, MCP, export, public
  shares, link previews, secret scanning and plaintext sync, through
  `SpaceMode::allows`. The desktop app runs search and MCP for those
  spaces locally.

| Option | Server can read | Search on server | Chosen |
| --- | --- | --- | --- |
| Per-member `X25519` recipients | ✗ | ✗ | ✓ |
| Shared space key held by the server | ✓ | ✓ | ✗ |
| Shared space key wrapped per member | ✗ | ✗ | ◐ |

A shared key wrapped per member makes removal harder, because the whole
space has to be re-keyed. Per-document recipients let removal touch only
headers.

## Consequences

- New services that fetch URLs, accept files or index content depend on
  `vellum-security` and call these functions. Review rejects a raw
  `reqwest::Client` in service code.
- The ASVS L2 self-assessment (`docs/security/asvs-l2.html`) references
  these decisions as evidence.
