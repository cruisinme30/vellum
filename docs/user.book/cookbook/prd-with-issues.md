---
id: 01M63CFE6FJKC6SYVC2573N2TF
title: A PRD with live issue chips
vellum: 1
---

# A PRD with live issue chips

**You will build** a product requirements document whose requirements
link to issues that show live status, with a roadmap and a risk matrix,
that stays the single source of truth while the team builds.

## Steps

1. Create a page from **New → From template → PRD** and title it "PRD: Policy simulator".
2. Fill in the facts panel: owner, status and target.
3. Write the requirements as a table. Select a requirement and choose **Create issue**: the issue is created in ENG and the table gets a live chip.
4. Add a roadmap with `/roadmap` and a risk matrix with `/risk`.
5. Request review from the reviewers in **⋯ → Request review**; set the status to verified when they approve.

As the team works, each chip shows the issue's current status, so the
PRD never needs a status update by hand.

## The finished page

`````markdown {corpus="page/cookbook-prd" live=false}
---
id: 01JB9D3E5F7G9H1J3K5M7N9P1Q
title: "PRD: Policy simulator"
icon: 📐
status: in-review
owner: priya
tags: [prd, gateway]
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
`````
