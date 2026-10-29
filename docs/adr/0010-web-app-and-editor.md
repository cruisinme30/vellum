---
id: 01M63DBJ68777XDB5CTM10Q43H
title: "ADR 0010: Web app shell, block editor and the block schema"
status: verified
owner: "@cruisinme30"
tags: [adr, web, editor, schema]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D58, D59, D60, D172
- Issues: #70 (block schema), #90 (app shell), #92 (block editor),
  #79 (themes), #128 and #129 (collection views); the Yjs binding refs
  #97 (collab rooms); #392 (API clients); #89 (pages on the server's
  drive)
- Pull requests: #262 (shell and editor), #378 (collection views), #417
  (sign-in and live collections)

:::

## Context

Brief section 13 asks for a Material 3 shell (navigation rail, page tree,
themes, a phone layout) and a block editor that covers every Vellum
block, with a slash menu, stable block IDs and real-time collaboration.
The server stores Markdown (ADR 0004); the editor works on ProseMirror
nodes. Both sides must agree on one block model, and the web app had to
start before the API and the collab server exist.

## Decisions

### D58: one block schema generates the Rust and editor models

- `schema/blocks.json` lists every node and mark with its attributes,
  content, Markdown mapping and slash entry. `cargo xtask schema`
  generates `crates/vellum-contracts/src/blocks.rs` and
  `web/src/editor/schema/blocks.gen.ts`; `--check` fails CI and the fast
  suite when either is stale.
- Why: a block added on one side only would silently drop content on
  the other.

### D59: the editor speaks ProseMirror JSON and Yjs, never Markdown

- Markdown conversion happens only on the server. The browser edits a
  Yjs document bound with `@tiptap/y-tiptap`; block IDs (`blk-` plus 8
  base32 characters, as `vellum_kernel::BlockId`) are assigned
  client-side by an extension that skips remote transactions and
  re-assigns duplicates.
- Why: one parser and serializer (ADR 0004) keeps the canonical form in
  one place, and client-side IDs work offline.

### D60: the web app runs on ports with mock adapters

- Views talk to a `WorkspaceStore` port and the editor to a
  `CollabConnector` port. Today they run on Northstar mock data and a
  BroadcastChannel connector that speaks the y-sync state-vector exchange
  between tabs; the connector keeps updates in the clients' local
  `DocumentCache` (ADR 0023). The real API and collab server replace the
  adapters, not the views.
- Themes are Material 3 tonal palettes computed in-house in a perceptual
  (CIE) colour space, with WCAG 2.2 AA contrast checked in tests. Strings
  use Fluent; routing uses react-router. The editor chunk loads lazily.
- Collection views (#128, #129) follow the same pattern: a
  `CollectionsStore` port mirrors the collections service's JSON (ADR
  0015), and a Northstar mock serves it until the client lands. The
  server filters, sorts and groups; the six layouts only draw them.
  Every pointer action in them has a keyboard path: grid keys and inline
  editors in the table, Alt+arrows and a "Move to" menu on cards.
- The server adapters share one HTTP client (#392). It sends the session
  cookie, a CSRF token and an idempotency key on every write, and turns
  the server's error JSON into one error type that keeps the request ID.
  When `GET /api/v1/auth/status` gets no answer, the app keeps the mock
  adapters (demo mode); otherwise every page needs a signed-in user.
- Against a server the page tree is the drive's (`/api/v1/drive`, #89):
  the person's My Drive, made on first use, and their shared drives, with
  folders as tree nodes. Changes show at once and are undone, with the
  request ID shown, when refused; creating a page and search answer with
  promises. Bodies come only over the collab socket, never REST. The
  editor joins the page's room at
  `/collab/{space_id}/{doc_id}` (ADR 0009, #415) over WebSocket, with the
  session cookie; the server admits only the app's own origin. The
  client speaks the room's y-sync protocol with a small codec of its own
  (no direct `lib0` dependency), binding the `default` XML fragment. The
  editor stays read-only until the server's peer list names it a writer,
  never from the self-reported presence state. A dropped socket
  reconnects with backoff (0.5 s doubling to 30 s, jittered); offline
  edits stay in the document and the sync exchange sends them on
  reconnect. Close code 4403 (access revoked) stops reconnecting. Each
  connection re-announces presence with a newer clock, so a room that
  still holds the old one lists the editor again. Done asks the room for
  a version and resolves on its answer.
- Why: the shell and editor could ship before the backend, and swapping
  in the server is a transport change only.

### D172: versions on idle and Done, history restores through the room

- The editor writes a version 30 s after its own last change, and on
  Done (button, Ctrl/Cmd+S). Done with nothing new is only acknowledged,
  so this timer and the room's own never make two copies. The bar says
  Saved, Unsaved changes, Saving or failed (retried on the next quiet
  spell); offline edits save once the socket is back.
- History lists the drive's versions (time read from the ULID), shows a
  version's Markdown as text, and restores it as a new version. The
  editor saves first, so the text it replaces stays in the history.
- A page text written through the API (a save, a restore) is merged into
  the page's room, opening it from its log if needed: the log is the
  room's source of truth, so otherwise the room would write its old text
  back. The file's text wins over log edits it never held. A page never
  edited live has no log and is left alone.

## Alternatives

- **Hand-written types on each side**: rejected; they drift (D58).
- **Markdown in the browser** (a JS parser): rejected; two parsers would
  disagree on edge cases (D59).
- **`y-indexeddb`**: rejected in favour of the shared `DocumentCache`, so
  one store owns offline data (D60).
- **Restore by rewriting the document in the browser**: rejected; the
  browser has no Markdown parser (D59), and other editors would not get
  it from the file (D172).
- **`@material/material-color-utilities`**: dropped; its ESM imports
  lack file extensions, which break under Node and the tests (D60).

## Consequences

- A new block means editing `blocks.json`, running `cargo xtask schema`,
  then adding its Tiptap extension; a parity test fails until it exists.
- Collections run on the API since #417, and pages and live editing on
  the drive and the collab socket; work, books and search move next
  (#392). The views stay unchanged.
- In demo mode pages keep their local cache. Against a server a page
  opens only once its room answers; an offline copy of server pages is
  later work (ADR 0023).
