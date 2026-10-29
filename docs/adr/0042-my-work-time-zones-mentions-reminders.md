---
id: 01M64PF4WWYDA67S91QH6JSVMG
title: "ADR 0042: My Work time zones, mentions and reminder delivery"
status: draft
owner: "@cruisinme30"
tags: [adr, my-work, tasks, notifications]
vellum: 1
---

::: facts

- Date: `2026-10-29`
- Status: accepted
- Decisions: `D178`, `D179`, `D180`, `D181`
- Issue: `#472`
- Pull requests: none yet (this branch)

:::

## Context

[ADR 0018](0018-my-work.md) built My Work on one
server-wide time zone, listed mentions only as checklist items naming
`@me`, and left reminders to "the notifications service, on a timer",
which nobody started. So a person abroad saw the wrong "today", a comment
that named someone never reached their list, and no reminder or digest
was ever sent (R-MW-1, R-MW-5).

## Decisions

### `D178`: each person has their own My Work settings

- `GET` and `PUT /v1/my-work/settings`: an IANA time zone (or `null` to
  follow the server), the start of day and the digest time (`null` is off).
- Zones are looked up in jiff's time zone database; unknown names, odd
  characters and anything over 64 bytes are refused. Times must be `HH:MM`.
- `Profile` carries the stored preferences and the zone in force, so
  "today", relative reminders and the digest all use the person's zone.
- Serve keeps them in `<data>/.vellum/tasks/people.json`; a saved zone that
  later stops parsing falls back to the defaults with a warning.
- The web form pre-fills the browser's zone, so one Save adopts it.

### `D179`: unanswered comment mentions are My Work items

- `Comments::mentions_of` lists open threads where someone else mentioned
  the person after their own last comment, one item per thread.
- They come out as `Source::Mention` (`kind: mention`) in the Inbox bucket,
  with the issue key or page and who wrote it (`mentioned_by`).
- They leave when the thread resolves, when the person replies, or on
  `POST /v1/my-work/mentions/{comment}/dismiss` (the last 500 per person
  are remembered).
- A mention on a page counts only while the person can open that page.
- A thread assigned to the person stays an assignment, not also a mention.

### `D180`: one replica sends reminders and digests on a schedule

- `TasksService::notices` gives one person's reminders and digest in a
  window; `Schedule` holds the next six minutes for everyone and fires
  what is due, in time order, moving a watermark past it.
- Everyone is planned every five minutes. People whose files changed
  through My Work or an editor save are planned again on the next 20 s
  tick, so a reminder set for two minutes away is not missed.
- It runs under the `tasks.reminders` lease through `Replica::every`
  ([ADR 0038](0038-cluster-change-feed-and-jobs.md)), on one replica.
- The watermark is saved in `<data>/.vellum/tasks/reminders.json`; a
  restart resumes from it but never catches up more than 15 minutes.

### `D181`: reminders reach people as notifications

- Each notice becomes a `reminder` notification for the dispatcher: inbox
  line, web push and (once it exists) e-mail, by the person's preferences.
- `reminder` is urgent, so it skips quiet hours and the e-mail delay.
- The digest is a notification with the day's counts that links to My
  Work, sent only when the day is not clear.

## Alternatives

- **One zone per workspace**: simplest, but wrong for distributed teams.
- **Scanning every person every tick**: exact, but reads every person's
  task files three times a minute.
- **A notice table in the index**: survives replica changes exactly; kept
  for later because reminders do not yet justify a migration.
- **Ticking off a mention resolves the thread**: would close someone
  else's discussion, so dismissing is per person instead.

## Consequences

- Planning still reads each person's files every five minutes, as Google
  Calendar sync does; very large deployments will want a reminder index.
- The watermark is per replica: when the lease moves, the new sender may
  repeat up to 15 minutes of reminders or skip none, never more.
- Mentions on documents need a drive lookup per page; results are cached
  per request only.
- A mention in page text outside a checklist item does not become a My
  Work item; only checklist items that name the person do.
