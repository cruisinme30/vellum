---
id: 01M63CFE6FA6D69FAA5YXV3RES
title: Meeting notes with action items
vellum: 1
---

# Meeting notes with action items

**You will build** a meeting note whose action items land in each
owner's My Work, with due dates on their calendars.

## Steps

1. Create a page from **New → From template → Meeting notes**, or a blank page titled "Gateway sync 2026-11-02".
2. Under **Decisions**, add a `good` callout for each decision so it stands out when people skim.
3. Under **Actions**, type `[]` to start a task, then `@` to mention the owner and a date.
4. Share the page with the attendees.

Each action with a person and a date now shows in that person's My Work
and calendar feed. When they tick it there, it ticks here.

## The finished page

````markdown {corpus="page/cookbook-meeting-notes" live=false}
---
id: 01JB9A0B2C4D6E8F0G2H4J6K8M
title: Gateway sync 2026-11-02
status: verified
owner: priya
tags: [meetings, gateway]
vellum: 1
---

::: facts

- Date: 2026-11-02
- Attendees: @priya @dev @maya @sam
- Next: 2026-11-09

:::

## Decisions

::: good
Ship the diff report behind a flag on 2026-11-24.
:::

## Notes

- Replay of 7 days now takes 14 minutes; the target is 10 (ENG-142).
- Design for the PDF report is ready for review.

## Actions

- [ ] Profile the replay reader @dev 2026-11-05
- [ ] Review the PDF report design @priya 2026-11-04
- [ ] Add the replay job to the on-call runbook @sam 2026-11-06
````
