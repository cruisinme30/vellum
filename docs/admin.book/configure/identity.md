---
id: 01M63CFE61A85MTD76GSEW6GB7
title: Sign-in, SSO and SCIM
vellum: 1
---

# Sign-in, SSO and SCIM

## Sign-in methods {#methods}

**Admin → Sign-in** turns on any mix of:

- email and password, with optional TOTP or passkeys
- OpenID Connect (Google Workspace, Microsoft Entra ID, Okta, Keycloak and any other OIDC provider)
- SAML 2.0

People turn on two-step verification in **Settings → Two-step
verification**: an authenticator app, passkeys, or both, plus ten
one-time recovery codes. In **Admin → Sign-in**, require it for admins
or for everyone who signs in with a password; people it covers set it up
at their next sign-in. If someone loses every method and recovery code,
**Admin → Users → Reset two-step verification** removes them and ends
that person's sessions.

Add OpenID Connect providers in **Admin → Sign-in → Single sign-on**:
a name, the issuer URL, the client ID and secret, and the email domains
whose people get an account on first sign-in. Register the redirect URI
the page shows with each provider. The sign-in page shows a button for
every provider that is on. Client secrets are encrypted with the
identity key and never shown again; leave the field blank to keep one.
A provider set with `VELLUM_OIDC_*` still works and shows as read-only.

With SSO on, you can require it for everyone except **break-glass
admins** (**Admin → Users → Allow password**), who can still sign in
with a password and a second factor if the identity provider is down.
Existing sessions keep working while the provider is down, and the
`vellum admin break-glass` code always works.

Servers that share one database (replicas) must share the identity key:
set the same 64-hex-character `VELLUM_IDENTITY_KEY` on each of them.

## SCIM provisioning {#scim}

Turn on SCIM in **Admin → Sign-in → Provisioning** and paste the URL and
token into your identity provider. Users and groups created there
appear in Vellum; users removed there are deactivated, their sessions
end and their content stays with its folders.

## Sessions {#sessions}

Browser sessions use HttpOnly, Secure, SameSite cookies and expire after
the idle time you set. Repeated failed sign-ins lock the account for a
period. **Admin → Users → Sign out everywhere** ends all of one person's
sessions at once.

## API tokens {#tokens}

People create personal API tokens in **Settings → API tokens**, each
with scopes (`read`, `write:draft`, `write:direct`, `work`, `admin`) and
an expiry; the token is shown once. In **Admin → API tokens**, admins
can list and revoke any token and cap the longest expiry allowed.
