---
id: 01JB9D3E5F7G9H1J3K5M7N9P1Q
title: "PRD: Policy simulator"
icon: 📐
status: in-review
owner: priya
tags:
- prd
- gateway
verify_every: 90d
vellum: 1
---

::: facts
- Owner: @priya
- Status: In review
- Target: 2027-01-10
- Project: Policy simulator
:::

## Problem

Policy changes ship blind: nobody knows which requests a new rule will
block until customers report it.

## Requirements

| # | Requirement | Issue |
| --- | --- | --- |
| R1 | Replay 7 days of traffic in under 10 minutes | ENG-142 |
| R2 | Diff report of allowed and blocked requests | ENG-143 |
| R3 | Share the report as a PDF | ENG-151 |

## Plan

```roadmap
range: 2026-10-01 .. 2027-02-28
Replay engine | 2026-10-01 | 2026-11-15
Diff report | 2026-11-01 | 2026-12-15
PDF report | 2026-12-01 | 2027-01-05
GA | 2027-01-10 | milestone
```

## Risks

```risk-matrix
Replay too slow for large tenants | likelihood 3 | impact 4
PDF layout slips | likelihood 2 | impact 2
```

::: details Out of scope
Simulating policies against synthetic traffic; live A/B of policies.
:::
