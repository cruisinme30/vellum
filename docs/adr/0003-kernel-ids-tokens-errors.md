---
id: 01M63ARF51ETPA47M8ZZV76TAH
title: "ADR 0003: Kernel IDs, share tokens and errors"
status: verified
owner: "@cruisinme30"
tags: [adr, kernel, foundation]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D37, D38, D39
- Issue: none (recorded before the issue rule, D119)
- Pull requests: #23

:::

## Context

Every crate depends on `vellum-kernel` for identity, share links and
errors. The brief fixes the broad shape (`UUIDv7/ULID` typed wrapper types,
`^blk-xxxxxxxx` block IDs, 22-char `base62` share tokens stored as `SHA-256`)
but leaves the encodings and the error model open. Store, index, format
and encryption are built in parallel against this API, so it has to be
settled now.

## Decisions

### D37: object IDs are `UUIDv7` values written as ULID text

- Every object ID is 128 bits generated in `UUIDv7` layout (48-bit
  millisecond time, version and variant bits, random bits).
- Text form, in front matter, URLs and JSON, is the 26-character ULID in
  `Crockford base32`, uppercase. Storage form is the same value as a
  Postgres `uuid` (or 16 bytes in SQLite). Both sort by creation time.
- Parsing accepts any ULID, case-insensitive, so files from other tools
  load; `I L O U` are rejected rather than aliased, so each ID has one
  spelling.
- One wrapper type per object kind (`DocId`, `SpaceId`, `IssueId` and so
  on); `ObjectId` is the generic front-matter `id` the watcher reads
  before it knows the kind, and converts to any typed ID.

### D38: block IDs are 40 random bits in lowercase `Crockford base32`

- A block ID is `blk-` plus 8 `Crockford` characters (40 bits from the OS
  `CSPRNG`), written `^blk-xxxxxxxx` in Vellum Markdown.
- Canonical form is lowercase; parsing is case-insensitive and rejects
  `i l o u`. Same alphabet as D37, so one decoder serves both.
- Block IDs only need to be unique within one document; the editor
  regenerates on the rare collision.

### D39: share tokens and the shared error model

- Share tokens are 22 `base62` characters (about 131 bits) drawn by
  rejection sampling from the OS `CSPRNG`. Only the `SHA-256` hash is stored
  and compared, in constant time. The token type has no `Display`, a
  redacted `Debug`, and is wiped from memory on drop. Expiry,
  revocation and passwords live on the share record (`ShareId`).
- Crates keep precise error enums and convert into the kernel `Error`
  at service boundaries. `ErrorKind` (invalid input, unauthenticated,
  permission denied, not found, conflict, rate limited, unavailable,
  timeout, internal) decides the HTTP status, MCP error code and retry
  policy. The message is safe to show; detail stays in the source chain
  for logs and traces.
- A malformed share token is reported as not found, so a caller cannot
  tell a malformed link from a revoked one.

## Alternatives

- **Plain ULID without version bits**: not a valid `UUID` for Postgres.
- **`UUIDv7` in hyphenated text**: longer and harder to read in front
  matter.
- **Storing share tokens in clear**: a database leak would leak every
  link.

## Consequences

- IDs reveal their creation time to the millisecond. An ID is never a
  capability, so that is acceptable.

- Other crates import IDs, tokens, errors, events, timestamps and config
  from `vellum-kernel` and define none of their own.
- Adding an ID type, event kind or error kind is non-breaking: the enums
  are `#[non_exhaustive]`.
