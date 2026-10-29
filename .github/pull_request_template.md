<!--
SPDX-FileCopyrightText: 2026 The Vellum contributors
SPDX-License-Identifier: AGPL-3.0-only

Write for a reader who knows nothing about this project's internal IDs.
CI fails when a required section is missing or empty, or when a decision
(D12) or record (ADR 0012) is named without a plain description and a
link. Write "None" with a reason when a section truly does not apply.
-->

## Issue

<!-- "Closes #N" (or "Refs #N" when the issue stays open). Release,
chore and ci pull requests are exempt. -->

## Why

<!-- The problem and who it affects, in plain words. What is wrong or
missing today, and what changes for people once this lands? -->

## What changed

<!-- The main changes, one bullet each, in plain words. Name a decision
with what it says and a link, for example: "Book chapters use lowercase
file names ([D67 in ADR 0013](docs/adr/0013-books.md))". -->

## How to verify

<!-- Tests added or run, and the steps a reviewer can follow to see the
change work. -->

## Security and operations

<!-- Effect on authorization, encryption, input handling or secrets, and
the logs, metrics or spans added. Or "None" and why. -->

## Screenshots

<!-- Optional: before and after, for visible changes. -->

## Checklist

- [ ] Branch is `<type>/<topic>`; title and commits are Conventional Commits
- [ ] Every commit is signed off (`git commit -s`)
- [ ] Public items documented; user guide updated if visible
- [ ] Linked issue holds the discussion; a feature's ADR in `docs/adr/` links it
- [ ] A reader with no project context can follow the Why and What changed
- [ ] Labels: `type/*` and `area/*` (set automatically), plus `phase/*`
