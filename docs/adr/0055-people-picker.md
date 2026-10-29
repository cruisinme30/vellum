---
id: 01M64WQ4ZAB0NXYNTZEF2KT3RX
title: "ADR 0055: People and group picker"
status: draft
owner: "@cruisinme30"
tags: [adr, web, identity]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D272
- Issue: #517
- Pull requests: none yet

:::

## Context

Book section access and publishing asked people to type `user:<id>` and
`group:<id>`. The editor share dialog and the admin screens need the same
choice. Listing users was an admin operation, so ordinary members had no
way to find a colleague.

## Decisions

### D272: a people directory and one picker

- `identity.directory` (`GET /api/v1/directory?q=`) lets any signed-in
  user search active people by name, handle or email. Queries under two
  characters find nobody, results stop at 20, and names that start with
  the query come first.
- Each entry is `{ kind, principal, name, detail }`; `principal` is the
  string access settings store, so forms keep what they save unchanged.
- `PeoplePicker` (`web/src/people/`) is the only picker: a WAI-ARIA
  combobox with chips, debounced search and typed IDs as a fallback.
  Named groups join through `combineDirectories` when their API lands.

## Alternatives

- **Reuse the admin user list**: it needs admin scope and fresh sign-in.
- **A picker per feature**: four widgets would drift apart.

## Consequences

- Members can see each other's names and emails, as in any shared
  workspace; the two-character minimum stops listing everyone at once.
- The share dialog and the admin screens import `PeoplePicker`.
