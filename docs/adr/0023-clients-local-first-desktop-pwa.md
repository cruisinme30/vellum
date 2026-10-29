---
id: 01M63BN51AA1NHNGC1HWNEMSPH
title: "ADR 0023: Clients: one UI, a local-first store, desktop and PWA"
status: verified
owner: "@cruisinme30"
tags: [adr, clients, desktop, pwa, local-first]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D97, D98, D99
- Issue: #185 (PWA); also #91, #182, #184 and #186
- Pull requests: #34, #139
- Requirements: R-WK-6, R-MW-5, R-LT-4, R-SE-10

:::

## Context

Vellum 1.0 ships three ways to use it besides the browser tab: an
installable web app (PWA) for phones, an Electron desktop app (macOS first
and notarised, then Windows and Linux), and the same web app with an
offline cache. The brief also asks for "sub-100 ms perceived latency via
the local-first store" and, for end-to-end encrypted private spaces, search
and MCP that run on the person's own device. The editor is the product, so
every client must run the same editor build.

## Decisions

### D97: one UI, three shells, one platform seam

- `web/` is the only user interface. The PWA is that app plus a service
  worker and a manifest. The desktop app is an Electron shell in
  `desktop/` that loads the same production build from a private
  `app://vellum/` scheme; it has no UI code of its own beyond native menus.
- Everything that differs per shell (sign-in, secure token storage,
  notifications, offline storage quota, deep links, local search and MCP)
  sits behind one `Platform` interface in `web/src/platform/`. The web
  implementation uses browser APIs; the desktop one calls a narrow,
  typed bridge exposed by the preload script.
- The renderer runs with context isolation, the sandbox on, Node
  integration off, a strict content security policy and no remote
  content in privileged windows. External links open in the system
  browser. The bridge exposes functions, never `ipcRenderer` itself.
- The PWA precaches the app shell with content revisions and serves it
  cache-first, so it starts offline. APIs, sign-in and MCP always go to
  the network; records reach the device only through the store (D98),
  never the HTTP cache. A new deployment offers a reload rather than
  swapping code under an open editor.
- Web Push uses VAPID keys held by the notifications service. Permission
  is asked only from a click; a payload links only to Vellum's origin.
- Layout, including the phone layout, belongs to `web/src/shell/`.
  Native iOS and Android shells (release 1.1) will reuse
  `web/src/platform/` unchanged.

### D98: the local-first store is push/pull with named mutators

- Structured records (issues, tasks, collection rows, tree nodes) live in
  an in-memory view backed by IndexedDB. A change is a named
  **mutator** with JSON arguments. The client runs it at once against
  the view, so the interface updates in the same frame, then appends it
  to an outbox on disk.
- Sync is a two-call contract served by the gateway:
  `POST /api/v1/sync/push` sends pending mutations in order, each with a
  per-client, monotonic ID that the server applies at most once;
  `GET /api/v1/sync/pull` returns record patches since an opaque cursor
  and the last mutation ID it has applied for this client. The client
  drops confirmed mutations and replays the rest on top of the new
  server state (rebase), so other people's edits merge under local
  intent. The server is the authority: it runs its own implementation of
  each mutator and may reject one (for example on a permission check),
  which the client drops and reports.
- Page content is not in this store. Pages are Yjs documents; the
  `DocumentCache` keeps their updates as opaque bytes in the same
  database so a page opens offline and the CRDT merges on reconnect. The
  web app keeps recent pages within a 50 MiB budget, evicting the least
  recently used; the desktop app pins whole spaces.
- One database per server origin and account; sign-out deletes it.
- Sync runs 50 ms after a change, every 30 s while idle and at once on
  reconnect, with jittered backoff to 60 s. 401, 403 and malformed
  responses stop sync and raise a banner.
- The client records `vellum.client.mutation.perceived_ms` and
  `vellum.client.sync.round_trip_ms`, with `traceparent` on every sync
  call, so perceived latency is measured, not assumed.

### D99: desktop sign-in, secrets, signing and private spaces

- Sign-in uses the system browser, never a web view: OAuth 2.1 with PKCE
  (S256) and a loopback redirect `http://127.0.0.1:<random port>/callback`
  (RFC 8252). The public client `vellum-desktop` has no secret.
- Tokens are encrypted with Electron `safeStorage`, which uses the macOS
  Keychain, Windows DPAPI, or the Linux Secret Service, and the ciphertext
  sits in the app's data folder. When Linux has no keyring
  (`basic_text` back end) the app keeps tokens in memory only and asks
  people to sign in again after a restart, rather than store a token
  that any local process could read.
- Releases are built with electron-builder. macOS builds are signed and
  notarised when the repository secrets `APPLE_ID`,
  `APPLE_APP_SPECIFIC_PASSWORD`, `APPLE_TEAM_ID`, `CSC_LINK` and
  `CSC_KEY_PASSWORD` exist. Pull requests build unsigned; a tagged
  release fails rather than publish an unsigned macOS build. Windows
  signing uses `WIN_CSC_LINK` and `WIN_CSC_KEY_PASSWORD` the same way.
  Updates use electron-updater against signed GitHub releases.
- E2E private spaces: the server stores only ciphertext, so the desktop
  app builds the search index for those spaces itself from decrypted
  blocks (BM25, block-level hits, the same ranking fields as the server)
  and keeps it encrypted at rest with a `safeStorage`-wrapped key.
  Decryption uses `vellum-crypto` compiled to WebAssembly behind a
  `Decryptor` interface.
- MCP for private spaces runs on the device: `Vellum --mcp` speaks MCP over
  standard input and output, as Claude Desktop and Claude Code expect, and
  forwards each call over a per-user local socket (mode 0600) to the
  running, unlocked app, which answers from its local index. Reads only;
  model-made writes to a private space become local drafts for review,
  as everywhere else.

## Alternatives

- **Replicache, Zero or ElectricSQL**: each ties the server to a
  separate service or to Postgres logical replication, which the single
  binary on SQLite cannot offer.
- **A separate desktop UI**: two editors to keep in step.
- **Sign-in in a web view**: RFC 8252 advises against it, and it exposes
  passwords to the app.

## Consequences

- One editor build behaves the same in the browser, the PWA and the
  desktop app; desktop-only features degrade to "not available" on the
  web through the `Platform` interface.
- Services that want offline writes register mutators on both sides with
  the same names; read-only features need nothing.
- The sync service must keep a per-client "last applied mutation" table
  and a change log per space with a cursor; resetting the log (for
  example after an index rebuild) answers pulls with `reset: true`.
- Notarisation waits only for the owner's Apple credentials; nothing else
  in the release path depends on them.
