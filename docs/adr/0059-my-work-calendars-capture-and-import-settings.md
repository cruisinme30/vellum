---
id: 01M650SD6826TXVYS7AN16VXMT
title: "ADR 0059: My Work settings for calendars, capture and import"
status: draft
owner: "@cruisinme30"
tags: [adr, my-work, web]
vellum: 1
---

::: facts

- Date: `2026-10-29`
- Status: accepted
- Decisions: `D298`
- Issue: `#525`
- Pull requests: none yet

:::

## Context

The server already serves the secret calendar feed, CalDAV, capture by
e-mail and Slack, and imports from Google Tasks, Todoist and Things
(R-MW-6, R-MW-8, [ADR 0018](0018-my-work.md)). The web app offered none
of it, so nobody could find their feed URL, capture address or an import.
Feed tokens and capture addresses are stored only as hashes, so the
server cannot show an existing one again.

## Decisions

### `D298`: one My Work settings page, secrets shown once

- `/my-work/settings` holds three sections: Calendar (feed URL, CalDAV
  URL, an app password), Capture (e-mail address, Slack status) and
  Import (pick the app, upload, preview the count, import, see the lists).
- `GET /v1/my-work/connections` returns the CalDAV account URL and whether
  e-mail capture, Slack capture and this person's Slack link exist.
- The feed URL, capture address and app password appear only when issued.
  Issuing a new one stops the old one; "turn off" revokes it.
- The CalDAV app password is an API token with `read` and `write:direct`.
- Slack links itself from "Save as task" in Slack; the page shows the
  state and offers Disconnect.
- Why: secrets stay hashed at rest, and each section maps to an existing call.

## Alternatives

- **Storing secrets to show them again**: weakens the feed and capture
  tokens for a small convenience. Rejected.
- **A server-side import preview**: the client count is enough to catch a
  wrong file, and the import itself is safe to run twice.

## Consequences

- People who lose a feed URL get a new one, and calendar apps must be
  updated with it.
- The import preview is a rough count; the server's reader decides.
- Rotating the feed makes the old URL return 404, covered by the service
  test `the_feed_needs_its_secret`.
