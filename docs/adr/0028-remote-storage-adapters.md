---
id: 01M63D3NB6KE65W6B6CZ2RW76Y
title: "ADR 0028: Remote storage adapters and the Git backend"
status: verified
owner: "@cruisinme30"
tags: [adr, storage, foundation]
vellum: 1
---

::: facts

- Date: `2026-10-29`
- Status: accepted
- Decisions: `D113`, `D114`, `D115`
- Issue: `#54` (S3), `#55` (Git), `#110` (Google Drive), `#111` (Dropbox and
  `OneDrive`)
- Pull requests: one for each issue

:::

## Context

The brief (sections 2 and 6) lists the backends behind the `BlobStore`
port: a local folder, S3, Google Drive, Dropbox, `OneDrive` and Git. Git is
first-class: one commit per save batch, and pulls go through the same
three-way merge as concurrent saves. The cloud drives are "eventually
consistent and always merged", and no backend may fall back to
last-write-wins. Every adapter must pass the storage adapter conformance
suite from `ADR 0005`.

Three constraints shaped the choices below. The workspace bans duplicate
crate versions (`cargo deny`, `multiple-versions = "deny"`). Vellum ships
as one binary, so each backend it does not use should cost nothing. And
the four HTTP backends differ widely in how they support conditional
writes.

## Options considered

- **Cloud SDKs** (`aws-sdk-s3`, generated Google and Microsoft clients):
  complete, but each pulls a large tree with its own HTTP and crypto
  stacks, which breaks the duplicate-version ban and grows the binary.
  Not chosen.
- **Apache `OpenDAL`**: one crate for all four services, and the brief's
  "`OpenDAL`-style" trait comes from it. It brings a second `digest` and
  `hashbrown` generation and its own conditional-write model, which would
  sit under ours. Not chosen; the trait stays `OpenDAL`-style.
- **Hand-written adapters over the `reqwest` client already in the tree**:
  more code to own, but each adapter maps its backend's conditional
  requests straight onto `Precondition`, and the dependency cost is
  one optional `hmac` for S3 signing. Chosen.
- **Git through `libgit2` or `gix`**: `libgit2` adds a C dependency and
  `OpenSSL`; `gix` cannot push yet. Running the `git` command-line tool
  uses the credentials the operator already set up. Chosen.

## Decisions

### `D113`: hand-written HTTP adapters, one cargo feature each

- S3 (`s3`), Google Drive (`gdrive`), Dropbox (`dropbox`) and `OneDrive`
  (`onedrive`) are modules of `vellum-store`, each behind its own feature,
  over the shared `remote` layer: one `reqwest` client with timeouts,
  retries on `429` and `5xx` that honour `Retry-After`, and one mapping
  from HTTP status to `StoreError`.
- S3 requests are signed with a small `SigV4` implementation. The cloud
  drives take bearer tokens from a `TokenSource`; `RefreshToken` trades a
  stored refresh token for short-lived access tokens. Secrets never appear
  in `Debug` output, logs or error messages.
- Why: no duplicate crate versions, and a deployment compiles only the
  backends it uses.

### `D114`: preconditions are atomic where the backend allows, merged where not

- S3 maps `IfAbsent` and `IfMatch` to `If-None-Match: *` and `If-Match`.
  Dropbox maps them to upload modes `add` and `update` with the file's
  `rev`. `OneDrive` maps them to `If-Match` and
  `conflictBehavior=fail`. These run the full conformance suite.
- Google Drive has no conditional writes, so the adapter checks the
  precondition just before writing. Two writers inside that window can
  both land. Drive keeps the overwritten revision, and the watcher and
  three-way merge reconcile it, so the folder is eventually consistent and
  always merged. Google Drive runs the suite with `Guarantees::BestEffort`,
  which skips only the racing-writers check.
- Each etag is the backend's own version marker: the S3 `ETag`, the
  Dropbox `rev`, the `OneDrive` `eTag`, the Drive head revision.
- Why: last-write-wins is ruled out, but refusing Google Drive would drop
  a backend the brief requires.

### `D115`: Git syncs by merge, never by rebase or force-push

- The Git backend is a work tree. Reads and writes go to the files, so the
  watcher and editors see a normal folder. `commit` records everything
  that changed, including edits made with other tools, as one commit per
  save batch. `.vellum/` is excluded from commits.
- `sync` commits, fetches the branch, and merges it. Files both sides
  changed go through the same three-way merge as a save: a clean merge is
  committed; a real conflict keeps the remote's version at the path and
  puts the local one in a `Title (conflict <date> <user>)` copy with a new
  identity; an edit beats a delete. Then it pushes, and merges again if
  the remote moved.
- History is never rewritten and never force-pushed, and repository hooks
  do not run. The `git` tool and its configured credentials do the
  network work.
- Why: merging keeps every other clone valid. The brief's "pull/rebase"
  is met by routing every conflicting file through the merge path; the
  only difference from a rebase is the shape of the history.

## Consequences

- Each new HTTP backend is a module plus a fake server for its tests, with
  no new crates.
- The S3 conformance test needs Docker (`LocalStack`). It fails in CI when
  Docker is missing and is skipped on developer machines without it. The
  cloud drives are tested against in-process fakes of their APIs, so a
  change in a real API shows up only in manual or nightly checks against
  real accounts.
- Google Drive users can briefly see two writers' versions; the merge
  path, not the backend, guarantees nothing is lost.
- The Git backend needs `git` in the runtime image.

## History

- `2026-10-29`: accepted; adapters built against issues `#54`, `#55`,
  `#110` and `#111`.
