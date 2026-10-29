---
id: 01M63CFE6BN3FM8WCNHE97ACE2
title: Cycles, triage and SLAs
vellum: 1
---

# Cycles, triage and SLAs

## Cycles {#cycles}

A cycle is a team's fixed working period, such as two weeks. In **Team
settings → Cycles** set the length, the start day, an optional cooldown
between cycles and how many future cycles to create ahead.

When a cycle ends, its unfinished issues roll into the next cycle,
except those in the backlog. Issues started or completed during a cycle
join it automatically. Each cycle shows a burn-up chart, and the team's
velocity is the average of the last three cycles.

## Triage {#triage}

Issues from outside the team (a form, an email, Slack, Sentry, another
team) land in the team's **Triage** inbox. For each one:

::: facts

- Accept (`1`): move it to the backlog or a status you choose
- Decline: cancel it with a reason that the reporter sees
- Duplicate: merge it into an existing issue
- Snooze: hide it until a date

:::

**Triage rules** run top to bottom on every new issue and can set the
team, labels, priority or assignee. Your own model can suggest a triage
decision, which you accept or change.

## SLAs {#slas}

An SLA sets how long an issue may take by priority. The defaults are 24
hours for urgent and one week for high. An issue's SLA marker turns
from gray to yellow, orange and red as the deadline nears, and the
assignee and team are notified before it breaches.
