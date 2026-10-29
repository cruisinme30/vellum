---
id: 01M63BFPJ7897ZRHP7CXT5MP9C
title: "ADR 0008: Authorization model and materialised access"
status: verified
owner: "@cruisinme30"
tags: [adr, authz, security, engine]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D52, D53, D54
- Issues: [#58](https://github.com/cruisinme30/vellum/issues/58), [#59](https://github.com/cruisinme30/vellum/issues/59)

:::

## Context

The brief settles authorization as Zanzibar-shaped ReBAC in the index:
grant tuples, a `page_ancestors` closure, materialised effective access
and an authorization epoch, deny by default, one indexed lookup per check. The
drive adds inheritance with overrides (a moved document keeps explicit
grants and drops inherited ones), shortcuts that never grant access, and
book sections that can only narrow access (R-DR-2, R-DR-3, R-BK-9). The
standards ask for permission changes visible within 1 s. The index tables
come from ADR 0006. This ADR fixes the rules that turn them into answers.

## Decisions

### D52: five ordered roles, checks read one row

- Roles are `viewer < commenter < editor < manager < owner`; each implies
  the ones below it. Actions map to the weakest role that allows them:
  read → viewer, comment → commenter, edit → editor, share and
  move/trash/purge → manager, transfer ownership → owner.
- `effective_access` holds each principal's **strongest** role per object.
  A check reads that one row by primary key and denies when it is absent.
  Search and MCP filter with one range read (`accessible`) or one batch
  read across spaces (`filter`).
- Principals are users and share links. Groups never appear in
  `effective_access`; they are expanded, transitively and cycle-safe,
  when rows are computed.
- A user can only change access on an object where they hold `manager`,
  and only `owner` can grant or revoke `owner`, so nobody hands out more
  than they hold. Server-internal writers (reindex, import, the expiry
  sweep) act as `System`.

### D53: inheritance, narrowing, moves and shortcuts

- A node's access is its parent's access merged with its own grants,
  strongest role winning; top-level nodes inherit from the space.
- A node marked `restricted` (a tuple `node#restricted@node`) narrows:
  it keeps only principals it grants explicitly, each capped at what they
  held above it, plus anyone who is `manager` or `owner` above it. Grants
  further down are capped the same way, so a restriction never adds
  access. Managers cannot lock themselves out.
- Grants are tuples on the node, so a moved node keeps them; inherited
  access is recomputed from its new ancestors when the index reports the
  move. Property tests check both halves.
- Shortcuts are ordinary nodes. Authorization never follows a shortcut to
  its target; opening the target is a separate check on the target.
- Group membership tuples live in `grants` under the all-zero `space_id`
  (the workspace directory), because groups span spaces. No extra table.

### D54: write-time materialisation under the epoch

- Every write (grant, revoke, restrict, membership, tree change) loads
  the space, applies the change in memory, recomputes the affected
  subtree, and commits tuples plus rows plus `authz_epoch + 1` in one
  transaction. The commit fails if the epoch moved since the load, and the
  engine retries, so concurrent writers never interleave stale rows.
- Because rows are written in the same transaction as the tuples, the
  next check on any node sees the change; the 1 s target is met by
  construction, and a test holds a 2,000-node write-plus-check under it.
- The epoch is returned with every check and exposed per space, so
  caches (render, search, MCP resources) key on it and drop entries the
  moment access changes.
- Expiring grants stay correct through a sweep job: `sweep_expired`
  deletes due tuples, re-materialises, and returns when the next sweep is
  due.

## Consequences

- Checks are constant-time and never walk the tree; writes cost a subtree
  (a space-level grant costs the whole space). Grant changes are rare next
  to reads, so this is the right side to pay on.
- `effective_access` grows as nodes × principals with access. Groups keep
  tuple counts small; the rows are what makes one-lookup checks possible.
- The engine talks to the index through the `AuthzStore` port. The
  in-memory adapter is the reference; the SQL adapter on `vellum-db`
  implements the same port and runs the same tests.
- Search and the drive call `filter`, `spaces`, `accessible` and
  `tree_changed`; they never read the grant tables themselves.
