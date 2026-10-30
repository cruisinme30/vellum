---
id: 01M652S57JG72N0R7JD3A9YA7C
title: "ADR 0067: Binary size and the busy memory budget"
status: draft
owner: "@cruisinme30"
tags: [adr, deploy, performance]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D279, D280
- Issue: #491, #187
- Pull requests: the integration batch that carries `perf/binary-size`

:::

## Context

The brief promises a 40 MB download and under 250 MB of memory with ten
live-editing rooms open and search warm (R-LT-1). Release 3 measured the
static musl binary at 100.1 MB and the compressed image at 40.7 MB, so
[ADR 0022](0022-deployment-and-release.md) set interim size budgets just
above those numbers. Nothing measured the busy memory figure. Discussion
is on [#491](https://github.com/cruisinme30/vellum/issues/491) and
[#187](https://github.com/cruisinme30/vellum/issues/187).

`cargo bloat` on a release build (glibc, without the 7 MB web app, 99.3
MB stripped) showed where the size goes: tract, the ONNX runtime for local
embeddings, was 12 MB that `vellum serve` never calls; the Mermaid
renderer was about 10 MB; sqlx pulled in ring next to aws-lc-rs.

## Decisions

### D279: the download is the gzipped binary, and every budget is checked

- "40 MB download" means the binary after `gzip -9`. That is how the
  `.deb` and `.rpm` packages, the image layers and compressed downloads
  carry it. `cargo xtask budgets` measures it and fails at 40 MB.
- The compressed image keeps release 3's 45 MB limit, and the binary on
  disk fails at 90 MB, a few MB above its size with the web app and the
  local search model, so growth still fails the check.
- The busy budget is measured on the same fresh server: the check sets up
  the first admin, mints an API token, creates a Northstar shared drive
  with 40 pages, runs 20 searches and opens ten collaboration sockets that
  each finish the Yjs sync handshake. It reads the resident set two
  seconds later and fails at 250 MB. The rooms stay open through SIGTERM,
  so the drain check covers live sockets.

### D280: size comes from features and the release profile

- tract sits behind `vellum-search`'s `onnx` feature, off by default. The
  crate enables it for its own tests through a self dev-dependency, so the
  ONNX tests still run with the workspace suite. Wiring a local model into
  `vellum serve` turns the feature on in `vellum-cli` and pays the 12 MB.
  Hybrid search (R-SR-5, #526) does exactly that, so the release binary
  carries tract; the gzipped binary stays under 40 MB, and the image keeps
  release 3's 45 MB limit.
- sqlx uses `tls-rustls-aws-lc-rs`, so the binary carries one crypto
  provider. Its trust roots are the same `webpki-roots` set as before.
- The release profile uses fat LTO instead of thin, still with one
  codegen unit and stripped symbols.
- The Mermaid crates (merman, dugong, manatee and helpers) build at
  `opt-level = "s"`. They run once per diagram.
- `panic = "abort"` is not used: tokio and axum turn a panic in one
  request into a 500, and abort would take the whole server down.

| Build (glibc, no web app) | On disk | gzip -9 |
| --- | --- | --- |
| Before | 99.3 MB | 39.1 MB |
| tract and ring removed, fat LTO | 75.7 MB | 31.7 MB |
| Mermaid at `opt-level = "s"` | 72.0 MB | 30.2 MB |

The last build measured 165 ms to ready, 57 MB idle, 0.4 % idle CPU and
103 MB busy. A page with a 25-node flowchart and a 15-message sequence
diagram renders in a median 9.7 ms instead of 8.9 ms.

## Alternatives

- **The raw binary under 40 MB**: would mean dropping features the brief
  requires, such as Mermaid rendering, SQLCipher or Postgres support.
- **`opt-level = "s"` everywhere**: slows search, sync and crypto, which
  are hot paths, for a few more megabytes.
- **UPX compression**: unpacks the whole binary into private memory at
  start, which breaks the 80 MB idle budget and confuses some virus
  scanners.
- **Compressing the embedded web app**: saves a few megabytes on disk but
  almost nothing gzipped, and needs content negotiation in the server.
  It can follow if the on-disk size matters.

## Consequences

- Fat LTO lengthens the final link of a release build by a few minutes
  (ADR 0066, on the release build, tracks build time).
- Adding local embeddings to the server costs 12 MB of the 40 MB, and the
  budget check shows it.
- The bare Linux binaries on the release page are not compressed. A
  `.gz` next to each would make the download match this budget; that
  belongs to the release workflow.
- The busy budget runs on every full-suite run, so a memory regression in
  collaboration or search fails the release.
