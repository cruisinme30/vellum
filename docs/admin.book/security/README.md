---
id: 01M63CFE617Z9J85T4D2WYD2JZ
title: Security overview
vellum: 1
---

# Security overview

Vellum is secure by default. You turn protections off deliberately,
never on.

::: cards

- 🔐 **Encrypted at rest** Every page has its own key; keys are wrapped by a key you hold.
- 🚫 **Deny by default** Nobody can open anything until someone grants access.
- 🧼 **Sanitised rendering** No raw HTML; every page is rendered through an allow-list.
- 📜 **Audit log** Every sign-in, share, admin change and AI call is recorded, append-only.
- ✍️ **Signed releases** Sigstore signatures, reproducible builds and an SBOM.
- 🧪 **Tested** An OWASP ASVS level 2 self-assessment and an external penetration test before every major release.

:::

## In transit {#transit}

Vellum serves TLS 1.3 with HSTS, and can get and renew its own
certificate with ACME. Public pages are served from a separate origin,
so a public page can never read a signed-in session. Responses carry a
strict Content Security Policy, and every form is protected against
cross-site request forgery.

## Uploads {#uploads}

Vellum checks the real type of each upload, enforces size limits, re-encodes
images to strip metadata and active content, and sanitises SVG. Set a
virus scanner hook in **Admin → Security → Uploads** to scan every file
with ClamAV before it is stored. Pages are scanned for secrets such as
API keys on save, and the author is warned.

## Reporting a vulnerability {#report}

Report vulnerabilities privately through the repository's security
advisories, as described in its security policy. Do not open a public
issue.
