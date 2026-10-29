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
- Pull requests: #242

:::

## Context

The repository is public and merges changes from many parallel threads
into `develop` with auto-merge and no human review (ADR 0001), so the
checks around the merge must catch what a reviewer would. The audit
in #239 found secret scanning, push protection, Dependabot security updates
and private vulnerability reporting on, but CodeQL on its default suite,
no generic-secret or validity checks, workflow tokens left on disk, no
workflow linting, no Dependabot cooldown, and rulesets that required
neither code scanning results nor signed commits.

## Decisions

### `D126`: workflows are linted and hold the least privilege

- Every action is pinned to a full commit SHA, and the repository
  setting requires it. The default workflow token is read-only.
- Every checkout sets `persist-credentials: false`.
- The Fast suite runs the zizmor workflow linter over `.github/` and
  fails on any finding of low severity or above.
- Privileged triggers carry an inline ignore with the reason: none of
  them runs the pull request's code.
- The Fast suite runs dependency review and fails on any new dependency
  with a known vulnerability. Licences and bans stay with `cargo deny`.

### `D127`: Dependabot updates are grouped, delayed and patch-merged

- Rust crates, npm packages and Actions each get one pull request for
  patch updates and one for minor updates; majors come one per package.
- A cooldown holds back versions younger than seven days (three for
  patches). Security updates skip it.
- A workflow turns on squash auto-merge for patch updates only; they
  still need a green Fast suite.

### `D128`: code scanning gates merges; Scorecard watches the repository

- CodeQL runs the extended suite on Rust, JavaScript and TypeScript, and
  Actions. Since 2026-10-29 it is an advanced setup (`codeql.yml`) that
  scans release pull requests, pushes to `main` and `develop` weekly
  ([ADR 0035](0035-ci-speed.md)); before that it was the default setup.
- The `main` ruleset requires CodeQL results and blocks on any high or
  critical security alert and any error-level alert (`develop` dropped
  this rule, see [ADR 0035](0035-ci-speed.md)).
- Rulesets require signed commits (GitHub signs the merges it makes),
  restrict `main` to merge commits, and protect `v*` tags.
- OpenSSF Scorecard runs weekly and uploads its findings to code
  scanning.
- Secret scanning also checks non-provider patterns and secret validity.

## Alternatives

- **Advanced CodeQL workflow**: first rejected for upkeep; adopted later
  to scan per release instead of per push ([ADR 0035](0035-ci-speed.md)).
- **Required approvals**: an author cannot approve their own pull
  request, so auto-merge would stop.
- **Strict status checks**: every merge would make every other open pull
  request stale; Develop health catches combined breakage instead.
- **Copilot code review**: fails on every pull request once the quota
  runs out; CodeQL and Scorecard are free on a public repository.

## Consequences

- A workflow change that zizmor flags fails the Fast suite; fix it or
  add an inline ignore with the reason.
- Release pull requests wait for CodeQL, and a high-severity alert
  blocks the release until fixed or dismissed with a reason.
- Release branches must be cut straight from `develop` with no extra
  local commits, because `main` requires signed commits.
- Settings outside the repository are recorded in #239. Revisit if the
  repository moves to an organisation, which unlocks a merge queue.
