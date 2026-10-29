<!--
SPDX-FileCopyrightText: 2026 The Vellum contributors
SPDX-License-Identifier: AGPL-3.0-only
-->

# Penetration test scope for Vellum 1.0

The brief (§11) requires an independent penetration test before 1.0, and
issue #195 tracks it. This page is the scope to send to the testing firm.
The OWASP ASVS level 2 self-assessment
([asvs-l2.html](asvs-l2.html)) lists the controls the test should verify.

## Objectives

1. Find vulnerabilities an outside attacker or a signed-in user could
   exploit, in the server, the web app and the desktop app.
2. Verify the ASVS level 2 controls the self-assessment marks as met.
3. Review the encryption design: envelope encryption at rest, KMS use and
   end-to-end encrypted private spaces.

## Targets

| Target | What it is | How the tester gets it |
| --- | --- | --- |
| Web app and API | `vellum serve`: REST and GraphQL API, sign-in, admin console | A staging URL with the Northstar demo workspace |
| Live editing | WebSocket at `/collab/{space_id}/{doc_id}` | Same staging URL |
| Public pages | Shared pages and share links on the separate pages origin | A second staging host name |
| OAuth and MCP | The OAuth 2.1 authorization server and the MCP resource server | Same staging URL; an MCP client such as Claude Desktop |
| Desktop app | The desktop app, with the local private-space agent and sync | Signed macOS build; Windows and Linux builds when signing certificates exist |
| Cluster deployment | Helm chart with several replicas, Postgres and S3 | Read-only review of `deploy/helm`; a live cluster is optional |
| Source code | The public repository at the release candidate tag | GitHub |

## Test accounts

Create these in the staging workspace before the test starts:

- Two organizations, so cross-tenant access can be tested.
- In each: an owner, an admin, a member, a guest limited to one space, and
  a user who has been removed.
- One end-to-end encrypted private space with two enrolled devices.
- One share link with a password and an expiry, and one public page.

## Areas of focus

- Authorization: access to spaces, pages, books, collections, issues and
  comments across roles and organizations; insecure direct object
  references in every API route; search results; WebSocket rooms after
  access is removed.
- Authentication and sessions: sign-in, lockout, OIDC single sign-on,
  break-glass admin, re-authentication for admin actions, session
  rotation and revocation, CSRF.
- Input handling: Markdown and diagram rendering (stored XSS), SVG,
  uploads (type confusion, archive bombs, polyglots), imports from Notion,
  Confluence and Word, link previews and webhooks (server-side request
  forgery).
- Cryptography: envelope format and key wrapping, passphrase key
  derivation, AWS KMS with role credentials, end-to-end space key
  enrolment and member removal.
- Collaboration protocol: malformed or oversized updates, replay, and
  access re-checks on open sockets.
- MCP and OAuth: token audience and scope checks, redirect URI matching,
  PKCE, consent.
- Deployment: container hardening, Helm defaults, secrets handling, TLS
  and security headers.

## Out of scope

- Denial-of-service floods against shared infrastructure. Our own load and
  hostile-traffic tests cover these (ADR 0047).
- Third-party services: AWS, Google Cloud, GitHub, the identity provider
  and the email provider themselves.
- Social engineering and physical access.

## Rules of engagement

- Test only the staging hosts and accounts listed above.
- Testing window: 5 working days, then a retest of the fixes within 30
  days.
- Report a critical finding to the contact below within 24 hours. Don't
  wait for the final report.
- Don't keep demo data or credentials after the engagement ends.

## Deliverables

- A report with each finding: description, steps to reproduce, impact,
  CVSS 4.0 score and a suggested fix.
- A short statement on the ASVS level 2 controls verified.
- A retest letter once fixes ship. Vellum publishes a summary with 1.0.

## What Vellum does with the findings

Each finding becomes a GitHub issue with a fix and a regression test, the
same way as the internal red-team findings (ADR 0030). Fixes ship before
1.0, and the ASVS page is updated in the same pull request.
