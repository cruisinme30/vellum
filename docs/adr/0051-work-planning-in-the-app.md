---
id: 01M64VFBEJE0GFZPFKHJ6JM59W
title: "ADR 0051: Work planning in the app"
status: draft
owner: "@cruisinme30"
tags: [adr, work, web]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D223, D224, D225
- Issue: #505
- Pull requests: to be linked when merged
- Extends: [ADR 0016](0016-work-issue-tracking-core.md)

:::

## Context

The Work engine already models projects, milestones, health updates,
initiatives, templates, insights and intake forms (R-WK-11, 12, 15 and 16),
but the web app showed only an issue list and an issue page. The service
served projects without their health or milestones, never loaded update or
initiative pages, and nothing ran Work's daily job in `vellum serve`.
Recurring issues existed only as an unused schedule model in
`vellum-integrations`. The 1.0 gap list (workstream 2) asks for the
planning screens, plus the prefilled create URL from R-WK-13.

## Decisions

### `D223`: planning is served from the files Work already owns

- Update pages (`projects/<slug>/updates/*.md`) and initiative pages
  (`initiatives/*.md`) load with the rest of Work and reload when another
  replica writes one, like a project.
- New routes: create a project from fields, add a milestone, list and post
  health updates, list, create and replace initiatives, and list a team's
  templates in full. Project and issue writes need `work`; reads `read`.
- A project reports its latest health, whether an update is due by its
  cadence, and progress per milestone. An initiative rolls up the worst
  latest health of its active projects and progress by points.
- Issues can be filed into and moved between milestones.

### `D224`: recurring issues live on the issue

- An issue with a due date may carry `repeat: 2w` (days, weeks or months,
  1 to 366) in its front matter, and a template may set one.
- Once the due date has passed in the team's time zone, Work files the next
  issue: same fields, the team's default status, the next due date that is
  not in the past. The repeat moves to the new issue, so a chain has one
  live issue and a missed run never floods the team.
- `vellum serve` checks every five minutes on one replica (`work.recurring`
  lease), so the issue appears just after 00:01 team time. The daily job
  (cycles, burn-up, SLA risk) now runs hourly on one replica (`work.daily`).

### `D225`: the planning screens are their own web area

- `web/src/work/planning/` holds the screens, a `PlanningApi` on
  `ServerApis`, its styles and its messages (`planning.ftl`, joined to the
  English bundle). The board and triage work (#504) owns the rest of
  `web/src/work/`.
- Routes: `/projects`, `/projects/:id`, `/initiatives`, `/insights`,
  `/asks`, `/asks/:team/:slug` and `/issues/new`.
- `/issues/new?team=ENG&title=…&template=…&repeat=1w` prefills the create
  form; nothing is created until a person submits it.
- A waiting AI triage suggestion shows on the issue page with Accept and
  Dismiss; the component is reusable in the triage inbox.

## Alternatives

- **Wire the `vellum-integrations` schedule model**: it supports weekdays
  and end dates, but keeps schedules apart from issues and needs its own
  storage and screens. A repeat on the issue is one front matter key that
  people can read and edit; richer rules can map onto it later.
- **Edit projects only as raw Markdown**: honest to the file, but slow for
  the common steps, so milestones and updates get their own forms.
- **A charting library for insights**: a table with inline bars is
  accessible, follows the theme and adds no dependency.

## Consequences

- Planning works end to end in the app; update and initiative files are
  plain pages that Git and other tools can read.
- The automatic "changes since the last update" list stays empty until the
  service keeps the project as it was at the previous update.
- The integrations schedule model is now unused for Work issues; it can be
  removed or adopted for weekday rules in a later change.
