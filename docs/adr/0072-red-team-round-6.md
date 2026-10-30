---
id: 01M65BZWG5QNAF99JQY42NCN3E
title: "ADR 0072: Red team round 6, the merged 1.0 build"
status: draft
owner: "@cruisinme30"
tags: [adr, security, red-team]
vellum: 1
---

::: facts

- Date: 2026-10-30
- Status: accepted
- Decisions: D315, D316, D317
- Issue: #529 (findings #547, #548, #550, #551, #552)
- Pull requests: to be linked when merged

:::

## Context

Round 6 of the red team ([ADR 0030](0030-red-team.md)) attacked develop
after batch #540 brought the day's work together: `vellum serve` wiring,
sharing, uploads and imports, AI and MCP, and sign-in with SAML, SCIM and
second factors ([ADR 0069](0069-sign-in-methods-second-factors-and-provisioning.md)).
Five findings came out of it, each fixed with a regression test. Two let
an identity provider, or whoever holds the provisioning token, reach
accounts it should not, one let a signed-in editor exhaust the server's
memory, one let a remembered provider sign-in pass the admin re-auth
check, and one kept MCP notifications flowing after access ended.

Checked and found sound: the SAML signature checks (one reference to
the signed element's own ID, unique IDs, no DTD, claims read only from
the verified assertion), second-factor tickets (one redemption, failures
count towards lockout, TOTP steps used once), share-link rendering on the
public origin, the page and block render routes, and link handling in
the web editor.

## Decisions

### D315: a provider reaches only accounts in its own domains

- Signing in through a provider links to an existing account with the
  same verified address only when the address is in the provider's
  allowed domains, if it lists any (RT-043). A provider without a domain
  list links as before. Identities already linked keep working.
- SCIM may rename, block and set the external ID of an admin, but never
  change an admin's address (RT-044), so the provisioning token cannot
  steer a provider sign-in into an admin account.

### D316: upload bodies held in memory come from one budget

- Every route that reads an upload into memory (attachments, file import,
  issue import) first reserves its `Content-Length` from one budget per
  process: 512 MiB by default (`VELLUM_UPLOAD_MEMORY_MIB`) and at most
  four uploads per person. Past either, the upload is refused with 503
  before its body is read (RT-045).
- The body is moved, not copied, into the upload check.

### D317: re-authentication and long-lived streams prove access again

- A SAML re-authentication records that it is one, and the assertion
  must carry an `AuthnInstant` from the last five minutes, the window
  OIDC's `auth_time` already has (RT-046). `ForceAuthn` alone is a
  request the provider may ignore.
- An MCP listen stream verifies its bearer token again at every
  keep-alive and before each notification, and closes with its result
  once the token fails (RT-047).

## Alternatives

- **Never link by address**: admins would have to link every provider by
  hand. Domain-limited linking keeps first sign-in simple and closes the
  takeover.
- **Keep SCIM away from admins entirely**: providers block leavers,
  admins included, and that should keep working.
- **A per-request body limit only**: each upload is already capped at
  100 MiB; the problem is how many run at once.
- **Streaming uploads to disk**: the checks (type sniffing, image
  re-encoding, the scanner) need the whole file; a budget is the smaller
  change for 1.0.

## Consequences

- A provider set up with allowed domains can no longer sign in people
  outside them, even if their accounts exist. Admins who relied on that
  remove the domain list or add the domain.
- Providers that do not send `AuthnStatement` cannot be used to
  re-authenticate; people sign in with a password or another provider.
- Large imports may wait for a free slot on a busy server.
- Regression tests: `crates/vellum-identity/tests/sso.rs`
  (`a_provider_cannot_link_accounts_outside_its_domains`),
  `tests/scim.rs` (`an_admins_address_is_out_of_reach`), `tests/saml.rs`
  (`reauthentication_needs_a_recent_authn_instant`),
  `crates/vellum-cli/src/uploads_tests.rs`
  (`uploads_are_refused_when_the_memory_budget_is_spent`) and
  `crates/vellum-mcp/tests/http.rs`
  (`listen_streams_close_when_the_token_is_revoked`).
