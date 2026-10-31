---
id: 01M63VWHC7QHKWVRQNVQPDJG0G
title: "ADR 0036: Search and AI tools for end-to-end encrypted spaces, on the device"
status: verified
owner: "@cruisinme30"
tags: [adr, desktop, crypto, search, mcp, privacy]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D147, D148, D149
- Issue: #365
- Pull requests: #381
- Requirements: E2E private spaces with client-side search and MCP
  (brief §9, §11)

:::

## Context

An end-to-end encrypted private space is stored as pages sealed to the
X25519 keys of its members' devices (ADR 0002). The server only ever holds
ciphertext, which is the point, but it also means the server's index and
its MCP server cannot see the space. A board member who keeps the board's
notes in a private space still wants to search them and ask Claude about
them, without the plaintext ever reaching the server or the disk.

The building blocks exist: recipient envelopes and the OS keychain
(`vellum-crypto`), the Markdown model (`vellum-format`), the in-memory
tantivy backend with a sealed index (`vellum-search`), and the MCP server
with its `Workspace` port (`vellum-mcp`). What was missing is the piece on
the device that holds the key, opens the pages, indexes them, and answers
an AI client. A new edge crate, `vellum-private`, does that, and the
`vellum` binary serves it as `vellum mcp --private`.

## Decisions

### D147: each device has its own key, kept in the OS keychain

- On first use a device creates an X25519 key pair and stores the secret
  in the operating system's keychain (macOS Keychain, Windows Credential
  Manager, the Linux Secret Service) under `dev.vellum.desktop` /
  `vellum-device-x25519-v1`. A stored value that is not 32 bytes is
  refused rather than replaced, so a damaged keychain never silently
  rotates the key.
- `vellum private key` prints the public key and a 16-byte fingerprint.
  A space owner enrols that public key; new pages are then sealed to the
  device too. Enrolment through the server is a later step.
- The secret never leaves the keychain and the process. The AI client
  launches `vellum`, so it never holds the key either.

### D148: pages are opened and indexed in memory only

- The agent lists each space's folder, opens every page sealed to this
  device, and builds block-level entries with stable IDs (the file's own,
  or one derived from the page and the block's position) and heading
  paths, so citations survive restarts.
- The index is tantivy in RAM, sealed with a random key made for this
  process alone. No plaintext, index file or cache is written to disk.
- A refresh rebuilds everything every 60 seconds by default, so a page
  resealed away from this device disappears from search on the next pass.
- Each pass counts what it skipped: pages sealed to other devices only,
  pages that should open but do not (tampered, not text, no ID), and
  plaintext files, which do not belong in a private space and are never
  trusted or indexed.

### D149: AI clients get read-only MCP over stdio

- `vellum mcp --private --space slug=path` serves the agent through the
  existing MCP server on standard input and output. The session gets the
  `read` scope only, so write tools are not offered.
- The `Workspace` adapter supports search, pages, blocks, outlines,
  space trees and listings. Every write returns `forbidden`; export is
  refused because it would write plaintext to a file; issues, books and
  collections return `not found`, since a private space holds pages.
- Citations are `vellum://doc/{doc}#{block}`.

## Alternatives

- **Search on the server over encrypted indexes** (searchable encryption):
  leaks access patterns and term frequencies, and no scheme we found
  supports ranked hybrid search.
- **Persist the index on disk, sealed with the device key**: faster
  start, but a second copy of the page structure on disk. Opening
  a space of a few thousand pages takes seconds, so we keep RAM only.
- **Let the model write drafts**: needs the device to seal each change to
  every other member's current key, which depends on enrolment. Deferred.

## Consequences

- Private spaces work with Claude Desktop, Claude Code and any stdio MCP
  client today. The desktop app can embed the same crate later.
- Headless Linux without a Secret Service cannot hold a device key; the
  command says so and exits.
- Server-side enrolment and endpoints that serve ciphertext to devices
  followed in ADR 0043 (#475).
