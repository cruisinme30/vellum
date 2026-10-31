---
id: 01M650HCFNV93B7HBQWZEZ1Z2J
title: "ADR 0069: Sign-in methods, second factors, SAML, SCIM and API tokens"
status: draft
owner: "@cruisinme30"
tags: [adr, identity, security, admin]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D290, D291, D292, D293, D294
- Issue: #523
- Pull requests: none yet

:::

## Context

The admin guide (`docs/admin.book/configure/identity.md`) promised SAML 2.0,
SCIM, optional TOTP or passkeys, an **Admin → Sign-in** page and a
**Settings → API tokens** page. None existed: single sign-on was one
`OpenID Connect` provider read from `VELLUM_OIDC_*` at start-up
([ADR 0020](0020-gateway-and-identity.md)), and the token API had no
screen. Identity owns the `OAuth` authorization server and every
credential, so all of this lives in `vellum-identity`; re-authentication
came from [ADR 0057](0057-admin-users-spaces-and-reauthentication.md).

## Decisions

### `D290`: TOTP and passkeys are second factors, with recovery codes

- People add an authenticator app (RFC 6238: SHA-1, 6 digits, 30 seconds,
  one step of drift) or passkeys (WebAuthn, ES256 or RS256, user presence
  required) in **Settings → Two-step verification**, and get 10 one-time
  recovery codes. Codes are stored as hashes, TOTP secrets sealed.
- With a factor, a correct password returns a five-minute ticket instead
  of a session; `POST /auth/sign-in/verify` redeems it with a code, a
  recovery code or a passkey. A TOTP step is accepted once.
- Wrong codes count towards the same lockout as wrong passwords, and a
  ticket dies after five wrong codes.
- The policy (off, admins, everyone) is set in Admin → Sign-in. Someone it
  covers without a factor gets an enrolment ticket: the session opens only
  after they add one.
- Re-authentication asks for a code too when the person has a factor.

### `D291`: personal API tokens get a page and an admin cap

- **Settings → API tokens** lists, creates (scopes within your own, an
  expiry) and revokes tokens; the secret is shown once.
- Admins list and revoke anyone's token under `/v1/admin/tokens` and cap
  the longest expiry (`PUT /v1/admin/token-policy`). Under a cap a token
  must expire; existing tokens keep their expiry.

### `D292`: sign-in settings live with the identity records

- One `sign_in` record (replicated like the others, ADR 0037) holds the
  methods, the second-factor policy, the providers, SCIM and the token cap.
- Several OIDC and SAML providers can be on at once; the sign-in page
  shows a button for each. Client secrets are sealed with the identity key
  (AES-256-GCM, `.vellum/keys/identity.key`) and never returned.
- A provider from `VELLUM_OIDC_*` still works and shows as read-only.
- "Require single sign-on" refuses passwords except for break-glass
  admins, who must also have a second factor. The break-glass code
  (`vellum admin break-glass`) always works.

### `D293`: Vellum is a SAML 2.0 service provider

- SP-initiated only: HTTP-Redirect `AuthnRequest`, HTTP-POST response to
  `/saml/acs`. Unsolicited responses are refused.
- The response holds exactly one assertion, and that assertion carries
  its own enveloped RSA-SHA256 or ECDSA-SHA256 signature over exclusive
  C14N from a certificate pinned in the provider settings (a response
  signature is checked too when present). SHA-1, keys inside the document
  and encrypted assertions are refused.
- The signature's one reference must name the signed element's own ID,
  IDs must be unique, and claims are read only from that element, so a
  wrapped copy is never consumed. Values holding comments are refused.
- A DTD is refused (no entity expansion); audience, recipient,
  `InResponseTo`, `NotBefore` and `NotOnOrAfter` are checked with two
  minutes of skew. Sign-ins in progress and used assertion IDs live in the
  replicated store until they expire, so any replica finishes a sign-in
  and a replay is refused everywhere.
- The relay state is a one-time secret, also set as a `SameSite=None`
  cookie on `/saml`, which binds the response to the browser that began.
- Metadata is read from a URL through the SSRF guard to fill in the form;
  the admin checks the values and saves. Canonicalization is tested
  against responses signed by another implementation (lxml).

### `D294`: SCIM 2.0 provisions people and groups

- `/scim/v2/Users` and `/scim/v2/Groups` (RFC 7644: list with `eq`
  filters, get, create, replace, patch, delete) take a `vsc_` bearer token
  that admins rotate in Admin → Sign-in; it is stored as a hash.
- SCIM users have no password and are never admins. `active: false` or a
  delete blocks the account, which ends its sessions and tokens; content
  stays where it is, and a blocked account still answers `GET`.
  Break-glass admins cannot be changed by SCIM, and the last admin is
  never blocked.
- SCIM groups are the admin groups of ADR 0057 (names unique ignoring
  case), with members changed through the permission engine; only active
  accounts can be added.
- The gateway leaves bearer tokens on `/scim/v2` to the SCIM router, so a
  SCIM token works nowhere else and no other token works there.

## Alternatives

- **A vetted SAML crate**: the maintained ones bind `libxml2` and
  `xmlsec1`, which would add C libraries to every build. A small verifier
  over `roxmltree` with the rules above covers SP-initiated sign-in.
- **SMS codes**: rejected; SIM swaps make them weaker than TOTP.
- **Settings in environment variables only**: rejected; admins asked for a
  page, and several providers do not fit one set of variables.
- **Separate SCIM tokens per provider**: one token per workspace is what
  the common identity providers configure.

## Consequences

- Sign-in becomes two steps for people with a factor; the web app and the
  re-authentication prompt ask for the code.
- Losing every factor and recovery code needs an admin to remove the
  factors from Admin → Users.
- SAML IdP-initiated sign-in is not supported; providers start at Vellum.
