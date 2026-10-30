<!--
SPDX-FileCopyrightText: 2026 The Vellum contributors
SPDX-License-Identifier: AGPL-3.0-only
-->

# Security policy

## Reporting a vulnerability

Report vulnerabilities privately through GitHub's private vulnerability
reporting: <https://github.com/cruisinme30/vellum/security/advisories/new>
(**Security → Report a vulnerability** on this repository). Do not open a
public issue.

Include the version or commit, the steps to reproduce, and the impact you
expect. Encrypted reports are welcome through the same form.

You will get an acknowledgement within three working days and an
assessment within ten. Fixes ship in a patch release, with a published
advisory that credits you unless you ask otherwise.

## Supported versions

Before 1.0 only the latest commit on `main` is supported. From 1.0, the
latest minor release receives security fixes.

## Disclosure

We follow coordinated disclosure. We aim to ship a fix within 90 days of
your report, and we publish the advisory, with a CVE where one applies, on
the day the fix is released: <https://github.com/cruisinme30/vellum/security/advisories>.

## How Vellum is secured

- The OWASP ASVS level 2 self-assessment:
  [docs/security/asvs-l2.html](docs/security/asvs-l2.html).
- Security decisions: [ADR 0025](docs/adr/0025-security-hardening.md) and
  [ADR 0032](docs/adr/0032-repository-security-controls.md).
