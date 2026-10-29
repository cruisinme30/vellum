---
id: 01M63DVWDKKBZ3D834GPSFF2HD
title: "ADR 0032: Repository security and quality controls"
status: verified
owner: "@cruisinme30"
tags: [adr, ci, security]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D126, D127, D128
- Issue: #239

:::

## Context

The repository is public, takes changes from 20 to 30 parallel threads,
and merges them into `develop` with auto-merge and no human review (ADR
0001). Nothing reads a change before it lands, so the controls around the
merge have to catch what a reviewer would. On 2026-10-29 Gourav turned on
GitHub's security features and asked for them to be made strict.

The audit in #239 found secret scanning, push protection, Dependabot
security updates and private vulnerability reporting already on. Gaps:
CodeQL ran the default query suite, secret scanning skipped generic
secrets and validity checks, checkouts kept the workflow token on disk,
nothing linted the workflows, Dependabot had no cooldown, and the rulesets
did not require code scanning results or signed commits.

## Options considered

- **CodeQL default setup or an advanced workflow**: an advanced workflow
  lives in the repository and can be reviewed, but it fails to upload
  while default setup is on and needs a rewrite whenever CodeQL changes
  its build modes. Default setup already covers Rust, JavaScript and
  TypeScript, and Actions, and needs no maintenance. Chosen: default setup
  with the extended query suite.
- **Required reviews**: one approval would stop the auto-merge flow,
  because the author cannot approve their own pull request. Rejected;
  automated checks stand in for review.
- **Strict status checks (branch up to date)**: every merge would make
  every other open pull request stale and stall auto-merge across the
  threads. Rejected; the Develop health workflow catches combined
  breakage instead.
- **Copilot code review**: it fails on every pull request once the
  monthly quota runs out and is not required. CodeQL and Scorecard cover
  security for free on a public repository. Turned off.

## Decisions

### `D126`: workflows are linted and hold the least privilege

- Every action is pinned to a full commit SHA, and the repository setting
  requires it. Workflows default to a read-only token; each job asks for
  what it needs.
- Every checkout sets `persist-credentials: false`.
- The Fast suite runs zizmor over `.github/` and fails on any finding of
  low severity or above. The three `pull_request_target` and
  `workflow_run` triggers carry an inline ignore with the reason: they
  never run the pull request's code.
- The Fast suite runs dependency review on every pull request and fails
  if it adds a dependency with a known vulnerability of any severity.
  `cargo deny` still owns licences and bans.

### `D127`: Dependabot updates are grouped, delayed and patch-merged

- Cargo, npm and Actions each get one pull request for patch updates and
  one for minor updates; majors come one per package.
- A cooldown holds back versions younger than seven days (three for
  patches), so a hijacked release is usually yanked first. Security
  updates skip the cooldown.
- A workflow turns on auto-merge (squash) for patch updates. They still
  merge only on a green Fast suite; minor and major updates wait for a
  person.

### `D128`: code scanning gates merges; Scorecard watches the repository

- CodeQL default setup runs the extended suite on Rust, JavaScript and
  TypeScript, and Actions. The `develop` and `main` rulesets require
  CodeQL results and block on any security alert of high severity or
  above, and on any error-level alert.
- OpenSSF Scorecard runs weekly and on every push to `develop`, uploads
  its findings to code scanning and publishes the score.
- Rulesets also require signed commits (GitHub signs squash and merge
  commits it creates), restrict `main` to merge commits, and protect
  `v*` tags from deletion and moves.
- Secret scanning also looks for non-provider patterns and checks
  whether found secrets are still valid.

## Consequences

- A workflow change that zizmor flags fails the Fast suite; fix it or add
  an inline ignore with the reason.
- Pull requests wait for CodeQL as well as the Fast suite. A high-severity
  alert blocks the merge until it is fixed or dismissed with a reason.
- Signed commits mean nobody can push to `develop` or `main` except
  through a pull request merged by GitHub. Release branches must be cut
  straight from `develop`, with no extra local commits.
- Settings that live outside the repository (rulesets, secret scanning,
  Actions permissions) are recorded in #239. Revisit if the repository
  moves to an organisation: a merge queue and organisation rulesets
  become available.

## History

- 2026-10-29: accepted in the pull request that closes #239.
