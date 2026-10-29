---
id: 01M64VQD838T5RR5F3SYBKTGJH
title: "ADR 0054: Editor collaboration surfaces"
status: draft
owner: "@cruisinme30"
tags: [adr, web, editor, collab]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D233, D234
- Issues: #93 (chips), #98 (presence and the two-browser test)
- Pull requests: none yet

:::

## Context

The editor turned typed tokens into chips (`@handle`, `[[Title]]`,
`ENG-142`, `2026-11-24`) through input rules only: a person had to know a
handle, a page's exact title or an issue key, and an issue chip showed the
bare key with no state. The workspace already lists people and pages, and
the work core answers issue lookups and title searches. Comments and issues
from a page are in [ADR 0010](0010-web-app-and-editor.md) (D232); this
record holds the other collaboration surfaces of the editor.

## Decisions

### `D233`: chip menus read the workspace, issue chips ask Work

- `@` opens a menu of people (handle or name) followed by dates: Today
  (kept as the moving `today` chip), Tomorrow, Next Monday and a typed
  `YYYY-MM-DD`. `[[` opens pages, then issues whose title contains the
  query or whose key it is. Both reuse the slash menu's list, so the
  keys and the ARIA wiring are the same.
- The host passes one `ChipSources` object for the editor's life. It reads
  the latest workspace on every keystroke, so the extensions are never
  rebuilt while someone types.
- An issue chip renders its team's name for the state after the key, and
  the title as its accessible name; a closed issue's key is struck
  through. Lookups are cached for a minute per key, so many chips of one
  issue make one request, and a failed lookup leaves the bare key.
- The Markdown never changes: chips still serialise as the plain token.
- Demo mode has no Work, so it offers people, dates and pages only.

### `D234`: presence shows only the peers the server vouches for

- The editor bar shows an avatar per other client in the room's awareness
  (initials on the person's caret colour, black or white ink by
  contrast), up to four and then "+N", each named "X is editing" or "X is
  viewing".
- Against a server a client appears only when the room's peer list names
  it, because the name in awareness is whatever that browser sends; the
  peer list carries the server's user and access (ADR 0009). Leaving the
  page drops the avatar with the next peer list.
- The Playwright test joins two browser contexts to one faked room that
  relays edits and presence, and checks text, avatars and carets.

## Alternatives

- **A server search for people and pages on each keystroke**: rejected;
  the workspace already holds both, and a round trip per key would lag.
- **Writing the issue's state into the document**: rejected; it would go
  stale and churn the Markdown on every state change.
- **`#` as the issue trigger**: rejected; it collides with heading input.
- **Trusting awareness names alone**: rejected; any browser could claim
  to be anyone in the avatar row.

## Consequences

- Any surface that embeds the editor passes `ChipSources`; without them
  the menus stay off and typed tokens still become chips.
- Issue chips cost one request per distinct issue a minute while a page
  is open.
