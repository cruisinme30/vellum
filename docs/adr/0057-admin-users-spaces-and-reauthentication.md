---
id: 01M64VPFNBGMGRN5GPVPK4NJ2N
title: "ADR 0057: Admin users, spaces and re-authentication"
status: draft
owner: "@cruisinme30"
tags: [adr, admin, security, drive]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D241, D242, D243
- Issue: #175
- Pull requests: none yet

:::

## Context

The admin console had requests, routes, audit, settings and backups
([ADR 0021](0021-observability-and-admin.md)), but no screens for users or
spaces. The identity service already served the user operations
(`/v1/admin/users`) behind a recent sign-in, and the gateway already
answered an old session with `401 reauthentication required` (R-SE-7).
Three things were missing: the web app had no way to answer that 401, an
admin could not see or change spaces they were not a member of, and admin
actions served by identity or drive were not in the audit log.

## Decisions

### `D241`: the web app asks the person to confirm who they are

- Every API call goes through `withReauth`: a `reauthentication required`
  answer opens a "Confirm it's you" dialog and the call runs once more after
  it. Calls waiting at the same time share one prompt.
- The dialog takes the password (`POST /auth/reauthenticate`). An account
  without a password goes through single sign-on with `reauth=1`, which
  makes the provider check the person again.
- Cancelling fails the call with the server's error. Sign-in itself uses the
  client without the retry.
- User and space admin operations need a sign-in from the last 15 minutes,
  queries included, so the prompt comes once on opening the screen.

### `D242`: admins manage spaces through the drive service

- `drive.admin_spaces`, `drive.admin_create_space`,
  `drive.admin_update_space` and `drive.admin_set_member` under
  `/v1/admin/spaces` need the `admin` scope and a recent sign-in.
- They skip the drive's member checks, because an admin is usually not a
  member; the gateway's scope check is the gate.
- `Access::members` and `Access::set_member` read and write role grants on
  the space itself. A new role replaces the old ones.
- A shared drive always keeps a manager or owner. A My Drive stays its
  owner's: admins change its name and limits, never its members.

### `D243`: the composition root audits every admin action

- `vellum_admin::audited` wraps each service so every `admin`-scope
  mutation appends one audit record: actor, operation, target ID and outcome.
- Inputs are never copied, because they can hold passwords. A failed audit
  write is logged and does not undo the change.
- `Operation::wrap` replaces a handler and keeps its declaration, so OpenAPI,
  `GraphQL` and the re-authentication window stay as declared.

## Alternatives

- **Prompt before opening any admin screen**: rejected; the server decides
  which calls need a fresh sign-in, and its window can change.
- **Spaces admin in the admin service**: rejected; a service never reads
  another's tables (brief §2), and the drive owns spaces and their grants.
- **Make every admin a member of every space**: rejected; it would show
  every page to every admin in search and lists.
- **Each service writes its own audit records**: rejected; identity and
  drive would depend on the admin crate, and new admin operations could
  forget to.

## Consequences

- Any operation declared with `fresh_auth`, such as API token creation,
  gets the prompt in the web app with no extra code.
- A request refused for an old sign-in is not audited, because the gateway
  refuses it before the handler runs.
- The `security.fresh_auth_minutes` setting does not yet change the
  windows, which are fixed when the operations are declared.
