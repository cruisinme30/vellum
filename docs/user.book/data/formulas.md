---
id: 01M63CFE6AKFSSD68KZ1A4J9VB
title: Formulas
vellum: 1
---

# Formulas

A formula property computes a value from the other properties of the
same row. Vellum computes formulas on the server when a row is saved,
so every reader, export and API client sees the same value. A formula
that fails shows its error in the cell instead of a value.

## The language

Refer to a property by name with `prop("Name")`. A name that is a
single word can be written bare: `Estimate` means `prop("Estimate")`.

::: facts

- Numbers: `8`, `0.5`, `-3`
- Text: `"Done"` in double quotes
- Booleans: `true`, `false`
- Arithmetic: `+ - * / %`
- Comparison: `== != < > <= >=`
- Logic: `and`, `or`, `not`
- Grouping: parentheses

:::

## Functions

| Function | Returns | Example |
| --- | --- | --- |
| `if(cond, a, b)` | `a` when `cond` is true, otherwise `b` | `if(Done, "✅", "")` |
| `round(n, places)` | `n` rounded; places default to 0 | `round(2.456, 1)` → 2.5 |
| `len(text)` | Number of characters | `len("ENG")` → 3 |
| `concat(a, b, …)` | The values joined as text | `concat("ENG", "-", 142)` → ENG-142 |
| `min(…)`, `max(…)` | Smallest or largest value | `max(Estimate, 1)` |
| `sum(list)`, `avg(list)`, `count(list)` | Over a list, such as a relation's values | `sum(prop("Points"))` |
| `now()` | The current date and time | `now()` |
| `year(date)` | The year of a date | `year(Target)` → 2026 |
| `days(a, b)` | Whole days from date `a` to date `b` | `days(now(), Target)` |
| `format(value)` | The value as text | `format(42)` → "42" |
| `percent(n)` | `n` shown as a percent | `percent(0.76)` → 76% |
| `empty(value)` | Whether the value is blank | `empty(Owner)` |
| `contains(text, part)` | Whether `text` contains `part` | `contains(Title, "API")` |
| `upper(text)`, `lower(text)` | The text in upper or lower case | `upper("eng")` → ENG |

## Errors

| Error | Meaning |
| --- | --- |
| `Unknown property "Estmate"` | A misspelled name; the message suggests the closest match |
| Expected a number, got text | Arithmetic on a text or select property |
| Division by zero | The divisor is 0 for this row |
| Circular reference | Two formulas refer to each other |

## Thirty worked formulas

Each formula is written for the requirements tracker in
[Collections](README.md) or the releases collection that relates to it.

| # | Goal | Formula | Result |
| --- | --- | --- | --- |
| 1 | Points left | `round(Estimate * (1 - Progress), 1)` | 1.5 |
| 2 | Done flag | `Status == "Done"` | ✓ |
| 3 | Traffic light | `if(Status == "Blocked", "🔴", if(Progress >= 0.8, "🟢", "🟡"))` | 🟡 |
| 4 | Days to target | `days(now(), Target)` | 12 |
| 5 | Overdue | `days(now(), Target) < 0 and Status != "Done"` | ✗ |
| 6 | Due soon label | `if(days(now(), Target) < 7, "⚠️ due soon", "🟢 on track")` | ⚠️ due soon |
| 7 | Issue key | `concat(upper(Team), "-", format(Number))` | ENG-142 |
| 8 | Percent done | `percent(Progress)` | 70% |
| 9 | Release progress | `percent(1 - prop("Points left") / prop("Total points"))` | 76% |
| 10 | Average estimate | `round(avg(prop("Points")), 1)` | 5.3 |
| 11 | Mean by hand | `round(sum(prop("Points")) / count(prop("Points")), 1)` | 5.3 |
| 12 | Largest item | `max(prop("Points"))` | 8 |
| 13 | Missing owner | `empty(Owner)` | ✗ |
| 14 | Needs triage | `empty(Owner) or empty(Target)` | ✓ |
| 15 | Mentions the API | `contains(lower(Requirement), "api")` | ✓ |
| 16 | Short title | `len(Requirement) <= 40` | ✓ |
| 17 | Priority weight | `if(Priority == "P0", 3, if(Priority == "P1", 2, 1))` | 3 |
| 18 | Weighted points | `Estimate * if(Priority == "P0", 3, 1)` | 24 |
| 19 | Target year | `year(Target)` | 2026 |
| 20 | Age in days | `days(prop("Created"), now())` | 27 |
| 21 | Stale | `days(prop("Created"), now()) > 90 and Status == "Open"` | ✗ |
| 22 | Cost | `Estimate * 950` | 7600 |
| 23 | Cost with buffer | `round(Estimate * 950 * 1.2)` | 9120 |
| 24 | Budget left | `Budget - sum(prop("Costs"))` | 4,200 |
| 25 | Over budget | `sum(prop("Costs")) > Budget` | ✗ |
| 26 | Even sprint | `Number % 2 == 0` | ✓ |
| 27 | At least one point | `max(Estimate, 1)` | 8 |
| 28 | Capped at 13 | `min(Estimate, 13)` | 8 |
| 29 | Status line | `concat(Status, " · ", percent(Progress))` | In review · 70% |
| 30 | Not done and not blocked | `not (Status == "Done" or Status == "Blocked")` | ✓ |
