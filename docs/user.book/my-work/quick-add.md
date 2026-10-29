---
id: 01M63CFE6DPBF9718KGFFWX7X2
title: Quick-add and recurrence
vellum: 1
---

# Quick-add and recurrence

## Quick-add {#quick-add}

Press `Q` anywhere and type a task in plain English. Vellum highlights
each part it understands as you type, so you can see what it will do
before you press `Enter`.

```text
Renew certs tomorrow 3pm p2 #ops +sam
```

| Token | Meaning |
| --- | --- |
| `tomorrow 3pm`, `next tue`, `in 3 days`, `nov 24` | Due date and time |
| `start:2026-11-03` | Start date |
| `every weekday 9am`, `every 2 weeks` | Recurrence from the schedule |
| `every! 2 weeks` | Recurrence from completion |
| `p1` to `p4` | Priority |
| `#list` | The list to put it in |
| `@label` | A label |
| `+person` | Assign to someone |
| `remind:2026-11-01 08:00` | A reminder |

## Recurrence {#recurrence}

`every` repeats on a schedule: "every Monday" is due every Monday
whether or not you finished last week's. `every!` repeats from
completion: "every! 2 weeks" is due two weeks after you last ticked it.

Vellum stores recurrence as an iCalendar RRULE, so it round-trips with
calendar apps.
