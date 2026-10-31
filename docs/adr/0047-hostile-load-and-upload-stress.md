---
id: 01M64QMF0FNXYCVTS5RE8S5BBR
title: "ADR 0047: Hostile load and upload stress"
status: verified
owner: "@cruisinme30"
tags: [adr, security, red-team]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D198, D199, D200, D201
- Issue: #478 (findings #479, #480, #481, #482, #483, #484, #485, #486)
- Pull requests: to be linked when merged

:::

## Context

Red team round 4 (ADR 0030) attacked two things: the server under
abusive clients, and the upload virus scanner under hostile files.

Against `vellum serve` built from develop, a request body trickled a
byte at a time and an HTTP/2 connection that only answers pings were
both held for as long as the client liked. `hyper`'s header timer covers
neither. Under the k6 attack mix, ordinary readers' p95 rose to 0.9 s.
Sign-in ran Argon2 (19 MiB each) with no bound beyond the 512-thread
blocking pool, and rate limits keyed on the full IPv6 address.

Against a real `clamd` with the shipped config, EICAR inside a zip nested
25 deep or inside an encrypted zip came back "OK", and a 150 KB zip that
inflates to 150 MB took 20 s of scanner time. serve reads
`VELLUM_CLAMD_ADDRESS`, but has no upload route yet, so nothing tied a
future route to the scan.

## Decisions

### D198: every connection has a deadline, not only its headers

- A request body may pause at most `header_read_timeout` (15 s) between
  chunks and must arrive whole within `body_timeout` (2 min).
- A connection with no request in flight for `idle_timeout` (1 min) is
  closed. A streaming response counts as in flight until its last byte.
- One client address may hold `max_connections_per_ip` connections.
  serve defaults it to 256 when it terminates TLS and to no cap behind a
  proxy (`VELLUM_TLS=off`), where every connection has the proxy's
  address. `VELLUM_MAX_CONNECTIONS_PER_IP` overrides it.

### D199: password checks are bounded, and IPv6 clients count per /64

- `password::Gate` lets two Argon2 checks per CPU run at once (2 to 64).
  A check that cannot start within 500 ms gets 429 and does not count as
  a failed attempt, so a flood cannot lock out its victims either.
- Rate limits, sign-in lockouts and the connection cap key a client by
  `rate::client_bucket`: IPv4 as is, IPv6 as its /64.

### D200: uploads go through one check, and the scanner fails closed

- `upload::check` is the only entry point for upload routes: size, type,
  archive and image checks on a blocking thread, then the scan of the
  bytes that are stored.
- ZIP containers, Office files included, are inflated before scanning:
  at most 200x per entry (8 MiB free), 512 MiB in all, 20,000 entries,
  4 levels deep. Encrypted, corrupt or unknown-compression archives are
  refused.
- At most 8 scans per replica are in flight; waiting counts against the
  30 s scan budget, so a flood is refused instead of queued.
- `clamd.conf` sets `AlertExceedsMax`, `AlertEncrypted` and a 30 s
  `MaxScanTime`, so `clamd` reports what it could not finish as found.

### D201: the attacks run every night

- `deploy/perf/k6/hostile.js` adds sign-in and token floods (with forged
  `X-Forwarded-For`), header bombs and 1 MB URLs.
- `deploy/perf/hostile/slow_clients.py` holds slow headers (`slowloris`), slow
  bodies and idle HTTP/2 connections, which k6 cannot, and fails if any
  stays open past its budget or a reader slows down.
- `crates/vellum-security/tests/it/clamd_stress.rs` runs against a real
  `clamd`: the config checks, and 56 hostile and clean uploads at once.
- All three run in the nightly workflow; the unit and server tests for
  each finding run on every pull request.

## Alternatives

- **Rely on the reverse proxy for slow clients.** The Personal and Team
  profiles can face the internet directly with ACME TLS, so the server
  must protect itself. Behind a proxy the limits are harmless.
- **A per-chunk body timeout only** (`tower_http` `RequestBodyTimeout`).
  A byte every few seconds never trips it; the total deadline is needed.
- **Close HTTP/2 connections at a fixed age.** Slots would still be held
  for that age, and long downloads would be cut. Idle time is the signal.
- **Let `clamd`'s limits decide on archives.** `clamd` answers "OK" past
  them unless told otherwise, and spends seconds getting there; checking
  in Vellum first is cheap and refuses bombs before they cost scan time.
- **Queue password checks instead of refusing.** A queue grows with the
  flood and holds blocking threads; a short wait then 429 keeps sign-in
  responsive for everyone else.

## Consequences

- A legitimate upload slower than 2 minutes for its whole body, or with
  a 15 s stall, is refused. 10 MiB in 2 minutes needs about 85 KB/s.
- An idle HTTP/2 client reconnects after a minute; browsers do this.
- Encrypted zips and archives nested more than 4 deep can no longer be
  uploaded, even clean ones.
- Under a sign-in flood some genuine sign-ins get 429 and retry; the
  server stays up.
- Each nightly run takes about 3 more minutes for slow clients, and the
  scanner job needs `clamd`'s first signature load (up to 15 minutes).
