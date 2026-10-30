---
id: 01M650K2T9SN0NSR9Z91PP9CQP
title: "ADR 0070: Red team round 5, the day's new features"
status: verified
owner: "@cruisinme30"
tags: [adr, security, red-team]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D281, D282, D283, D284
- Issue: #529 (findings #530, #531, #533, #534, #535, #536)
- Pull requests: to be linked when merged

:::

## Context

Round 5 of the red team (ADR 0030) read the features that landed or queued on
2026-10-29: uploads and export/import in `vellum serve`, admin users, spaces and
groups with re-authentication, bring-your-own-model assist, saved Work views,
MCP in serve, books publishing and themes. Six findings came out of it, share
links included, each fixed with a regression test. Two let one user act on
another's data (an import taking over page IDs, a public site copying a book's
snapshots), one skipped the admin re-auth check, and three were abuse of the
server's resources: two by any signed-in user, and one, guessing share-link
passwords, by anyone holding a link.

## Decisions

### D281: files that arrive from outside never keep an ID the index holds

- `index_tree`, which indexes copies and imports, treats every ID the
  index already holds as taken, so such a file gets a fresh ID (RT-037).
- The watcher's rescan already did this for IDs of other spaces; now
  every path that indexes new files does.

### D282: a credential's sign-in time is the person's, not the token's

- An `OAuth` grant records `auth_time`, the approving session's last
  sign-in, and its access tokens carry it (RT-039), like OIDC
  `auth_time`. Grants stored before fall back to `created_at`.
- Personal API tokens keep their creation time, which already needs a
  recent sign-in.

### D283: an admin-listed model endpoint is trusted at its exact base

- `VELLUM_AI_ENDPOINTS` entries match on scheme, host, port and path,
  never a path prefix, so users cannot reach other APIs on that host
  (RT-038).
- Model base URLs with a query, fragment or user info are refused.

### D284: what users and visitors can make the server do is bounded

- Refused saved views carry an owned reason; nothing is leaked to make a
  `&'static str` (RT-040). Filters take at most 16 KiB and a person owns
  at most 200 views.
- A book publication never copies book data: anything under a dot folder
  (`.snapshots`, `.history`, `.published`) or `book.yaml` (RT-041).
- A share link that takes 10 wrong passwords in 15 minutes stops
  checking passwords until the window passes, and link passwords are
  hashed behind the same `password::Gate` as sign-in (RT-042).

## Alternatives

- **Strip IDs from every imported file in the importer.** It would cover
  only importers that know about each Vellum file type; checking at
  indexing time also covers copies and future importers.
- **Require a fresh sign-in to approve an admin-scope OAuth client.**
  Useful, but tokens of older grants would still count as fresh for 15
  minutes; carrying `auth_time` fixes every grant.
- **Allow-list asset extensions for publications.** It would break
  legitimate attachments and still let `.json` snapshots through.

## Consequences

- An import or copy costs one index lookup per item it indexes.
- Admins who approved an `OAuth` client long after signing in must sign
  in again before its tokens can perform admin actions.
- A self-hosted model must be called at the exact base the admin lists;
  a user who typed a sub-path gets "can't reach".
- A 201st saved view is refused until the person deletes one.
- Someone guessing a link's password locks it for everyone for up to 15
  minutes; the owner can make a new link.
