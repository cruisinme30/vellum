---
id: 01M63CFE6GPKC1SA40WNMM5EYV
title: Release notes from an MCP prompt
vellum: 1
---

# Release notes from an MCP prompt

**You will** ask Claude to draft release notes from the issues
completed in a cycle, review the draft in Vellum and publish it.

## Steps

1. Connect Claude Desktop to Vellum with the `write:draft` and `work` scopes, as in [Connect Claude](../ai/README.md).
2. In Claude, ask: "Draft release notes for ENG cycle 2027-C04 in the Release notes folder. Group by feature, fix and chore, and link each issue."
3. Claude calls `list_changes` and `search` to find the completed issues, then `create_draft` to write the page.
4. In Vellum, open the draft from your Inbox. It shows "Drafted by Claude for Priya Shah".
5. Edit what needs editing, then select **Accept**.

The page is now an ordinary page, with Claude's draft and your edits in
its history.

## What Claude drafts

`````markdown {corpus="page/cookbook-release-notes" live=false}
---
id: 01JB9H7J9K1M3N5P7Q9R1S3T5V
title: Release notes, cycle 2027-C04
status: draft
owner: priya
tags: [release-notes]
vellum: 1
---

::: info
Drafted by Claude for @priya from 14 issues completed in 2027-C04.
:::

## Features

- Replay seven days of traffic in under ten minutes (ENG-142)
- Diff report of allowed and blocked requests (ENG-143)

## Fixes

- Rate limits no longer reset when a replica restarts (ENG-158)

## Chores

- Upgraded the TLS library (ENG-160)
`````

## Why it is safe

Claude acted with Priya's permissions, so it only read issues she can
read. With `write:draft` it could not change any page directly, and
every call it made is in the audit log.
