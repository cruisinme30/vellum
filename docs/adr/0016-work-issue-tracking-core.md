---
id: 01M63CERVEAA6YXZF0GJ5Q15GF
title: "ADR 0016: Work issue tracking core"
status: verified
owner: "@cruisinme30"
tags: [adr, work, issues]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D76, D77, D78
- Requirements: R-WK-1, R-WK-2, R-WK-3, R-WK-4, R-WK-5, R-WK-6, R-WK-8, R-WK-9, R-WK-11, R-WK-12, R-WK-15, R-WK-16
- Issue: #133, #134, #135, #136, #137, #138, #141, #142, #143, #144, #145, #146
- Pull requests: #207, #282, #290, #295

:::

## Context

Brief section 8 settles what Work does (D15 to D18): teams with key
prefixes, one file per issue, custom statuses in seven fixed categories,
cycles that run themselves, triage with rules and SLAs, projects with
health, views and My Issues. It leaves open where each object lives on
disk, how identifiers stay unique when issues move, how SLA risk is
measured, and how the engine is shaped so Git automation, the API, MCP
and the jobs service can all drive it. `vellum-work` (engine layer)
answers those.

## Decisions

### D76: Work files and identifiers

| Object | File | Notes |
| --- | --- | --- |
| Team | `teams/ENG/team.yaml` | workflow, estimates, cycles, triage rules, SLA rules, labels, templates, `last_number` |
| Issue | `teams/ENG/issues/ENG-142.md` | every field in front matter, body in Vellum Markdown |
| Cycle | `teams/ENG/cycles/2027-C04.yaml` | dates and the daily burn-up points |
| Project | `projects/<slug>/project.md` | fields in front matter, spec as body |
| Project update | `projects/<slug>/updates/<date>-<id>.md` | health, notes, changes since the last update |
| Initiative | `initiatives/<slug>.md` | projects in order |

- The alias table is not a separate file. An issue that moves team keeps
  its old identifiers in its own `aliases:` key, so the table is rebuilt
  from issue files like every other index.
- Numbers are never reused. The next number is one above the highest of
  `last_number` in `team.yaml`, every current identifier and every alias,
  so a lost or merged `team.yaml` cannot hand out a number twice.
- Cycle keys are `<year>-C<nn>`, numbered by start date within the year.
- Front matter keys the engine does not know are kept and written back
  unchanged, so a newer server or a hand edit never loses data.
- Label references are names (`gateway`, `type/bug`); a group allows one
  label per issue. Status references are slugs (`in-review`).

### D77: SLA risk and focus order

- SLA rules are filters evaluated top to bottom on every create and
  update; the first match starts the clock. Defaults: urgent 24 hours,
  high 168 hours. Lowering the priority out of every rule removes the
  clock; closing stops it and records whether it was met; reopening
  resumes it with the original deadline.
- Risk is the share of time used: gray below 50 %, yellow from 50 %,
  orange from 75 %, red at the deadline. Orange notifies the assignee;
  red notifies the assignee and the team lead. Each step is reported
  once (`notified:` in the clock).
- My Issues focus order is closed issues last, then the first that
  applies: urgent, running SLA (soonest deadline first), blocked by an
  open issue, in the current cycle, other active, triage, backlog. Ties
  sort by priority, then most recently updated.

### D78: engine shape

- `WorkGraph` holds teams, issues, cycles and projects in memory. Each
  command validates, applies, and returns `Changes`: the files to write,
  paths to remove, and `WorkEvent`s to publish. It never touches storage,
  authorization or transport, so every rule is a plain unit test.
- One structured filter (`and`/`or`/`not` over field conditions, with
  `@me`, `@today±Nd/w` and `@current`) serves views, triage rules and SLA
  rules. There is no query language in the UI (D15).
- Triage rules run before numbering, so a rule that routes an issue to
  another team does not burn a number in the first team.
- Cycle automation is one daily call per team time zone: create the
  current and upcoming cycles, roll unfinished issues on (unstarted and
  started move, backlog and triage leave, closed stay), record burn-up.
  A team that paused cycles restarts at today rather than back-filling.
- Integrations consume `IssueSnapshot` inside each event and drive the
  engine through `set_state`, `update_issue` (links), `create_issue`
  (intake) and `move_issue`.
- Work object IDs (`ProjectId`, `MilestoneId`, `ProjectUpdateId`,
  `InitiativeId`, `ViewId`) join the kernel's ID list, so no crate
  defines its own.

## Alternatives

- **A separate alias table file**: one more file to lose or conflict;
  aliases in the issue file rebuild like any other index.
- **Number from `last_number` alone**: a lost or merged `team.yaml`
  would hand out a number twice.
- **A query language for views and rules**: the brief (D15) rules it
  out; one structured filter serves all three.

## Consequences

- The service layer owns persistence: it loads every Work file into a
  `WorkGraph` at start (the rebuild path), writes the files named in
  `Changes` through the store, and publishes the events.
- Comments with reactions, Git automation, recurring issues, the API,
  imports and insights build on this crate in their own workstreams.
- Insights (#145) read only the lifecycle timestamps: lead, cycle and
  triage time as nearest-rank p50, p85 and p95, plus throughput and
  created counts, by one pivot and per week or month, over at most two
  years. Pivots use current values, such as today's assignee.
- Intake forms live in `team.yaml` and file their answers as triage
  issues. An assistant's triage suggestion is stored on the issue
  (`triage_suggestion` in its front matter) and applies only when a
  person accepts it, so a model never changes the tracker by itself.
- The keyboard model lives in the web app (`web/src/keyboard`): one
  table of commands feeds the shortcuts, the ⌘K menu and the `?` sheet,
  and key handling is a synchronous lookup to stay inside the 100 ms
  budget for perceived latency.
