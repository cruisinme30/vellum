---
id: 01M64QE56XDHBNVZHQMXSBVVH0
title: "ADR 0043: Enrolling devices in private spaces through the server"
status: verified
owner: "@cruisinme30"
tags: [adr, desktop, crypto, e2e, privacy]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D182, D183, D184, D185
- Issue: #475
- Pull requests: to be linked when merged
- Builds on: ADR 0002 (recipient envelopes), ADR 0025 (key registry and
  E2E policy), ADR 0036 (private agent, #365)

:::

## Context

ADR 0036 put search and MCP for end-to-end encrypted private spaces on the
member's device, but left two things out: enrolling a device through the
server, and handing ciphertext to devices. Until now a space owner had to
collect public keys by hand, and a private space only reached a device by
sync or a shared drive.

`vellum_security::e2e::KeyRegistry` already holds the rules for a space's
keys, and the sealed store already passes recipient envelopes through
untouched. What was missing is where the key list lives, who may change
it, how devices fetch and upload pages, and how existing pages follow when
a device joins or leaves.

## Decisions

### D182: a space's key list is a system file in the blob store

- Each private space's device keys live at `.vellum/e2e/<space id>.yaml`:
  member, public key, label and enrolment time per device. Public keys
  only; nothing secret is stored.
- It sits beside the content, so every replica sees the same list, backups
  and restores carry it, and no database migration is needed. Writes are
  compare-and-swap on the file's version, retried a few times, so two
  devices enrolling at once both land.
- `.vellum/` is never indexed, listed by the drive or served by the e2e
  API, so the list cannot be overwritten through the object endpoints.
- A space with an empty list is not private. Every change is checked by
  `KeyRegistry`: no weak keys, no duplicates, at most 255, never the last.

### D183: members enrol their own devices, managers start and prune

- `POST /v1/spaces/{id}/e2e/keys` enrols the caller's device. The first
  key makes the space private, so it needs manage access; after that any
  member who can view the space enrols their own devices.
- `DELETE …/keys/{fingerprint}` revokes one's own device, or anyone's for
  a manager. The answer names the removed key so devices strip it.
- Both need a sign-in from the last 10 minutes. A space the caller cannot
  see reports not found, so the API does not reveal which spaces exist.
- Access comes from the drive's own permission port, so private spaces
  follow the same sharing as every other space.

### D184: the server serves and accepts only ciphertext

- Devices list a space's objects, download them and upload pages they
  sealed (`…/e2e/objects`, `…/e2e/object`), as base64 inside the typed
  API, so the OpenAPI document, GraphQL and the TypeScript client cover
  them like every other operation. Envelopes are capped at 7 MiB.
- Only recipient envelopes are served, plus the space's own folder
  manifest. A readable page is refused with `conflict`: the drive serves
  it under its own checks.
- An upload must be a recipient envelope sealed to exactly the space's
  current keys. A device that missed a new member, or kept a revoked one,
  is told to refetch the keys and reseal. Writes state the version they
  replace, or create only, so nothing is lost to last-write-wins.

### D185: devices enrol and reseal with the bundled binary

- `vellum private enrol` and `vellum private reseal` talk to the API with
  the token in `VELLUM_TOKEN`. Reseal opens each page this device can
  read and seals it to the current keys with a fresh file key, so a
  revoked device cannot read later versions even if it kept the old one.
- `vellum mcp --private --server` reads spaces through a read-only
  `BlobStore` over the API, so the private agent works against a server
  as it does against a folder. Only ciphertext crosses the network.
- The desktop app runs the same binary from its main process, passing
  the token through the environment, never the command line. The web app
  gets a Private spaces screen; outside the desktop app it says that
  enrolling needs it.

## Alternatives

- **A database table per space**: needs a migration and a second place to
  back up, and gains nothing over a small file with compare-and-swap.
- **A raw byte endpoint instead of base64 in JSON**: a third less traffic,
  but outside the typed contract, so no OpenAPI, GraphQL or generated
  client. Pages are small; we can add one later if it matters.
- **The server re-wraps envelopes when a device joins**: impossible
  without a key that opens them, which is the point of E2E.
- **Add recipients without re-encrypting** (`add_recipient`): cheaper,
  but cannot drop a revoked device whose fingerprint is all we know, and
  keeps the old file key. A full reseal handles both.

## Consequences

- A member can make a shared drive private, enrol devices and search it
  from Claude without the folder reaching the device by other means.
- Trust in the key list rests on the server, as it does for sign-in. A
  malicious server could enrol its own key; members can compare
  fingerprints on the Private spaces screen. Pinning keys on devices is
  follow-up work.
- The drive still accepts plaintext writes into a private space's folder,
  and existing pages are not converted. Refusing server-readable writes
  in private spaces, and a conversion tool, are follow-ups.
- Reseal must run on a device that can open the pages after each enrol or
  revoke; the screen and the command say so.
