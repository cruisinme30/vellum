---
id: 01M639NFH15EBMG6C7NX1ZK4KA
title: "ADR 0001: Repository skeleton, branches and CI gates"
status: verified
owner: "@cruisinme30"
tags: [adr, ci, foundation]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D31, D32, D33, D112, D119

:::

## Context

The project brief (v2.0, section 14) asks for a first pull request that is
the repository skeleton with every CI gate enforced on an empty codebase.
After the brief, the product owner set a branching model: work lands on
`develop` behind fast tests, and `develop` merges into `main` behind the
full suite.

## Decisions

### D31: two long-lived branches, two suites

- All work branches from and merges into `develop`; `main` only receives
  `develop` (or a hotfix) and is where releases are tagged.
- The **fast suite** gates `develop`: formatting, every linter, unit and
  property tests, crate boundaries, supply-chain checks and the commit
  rules.
- The **full suite** gates `main` and runs nightly: the fast suite plus
  coverage floors, the release build, end-to-end and accessibility tests,
  and external link checks. Golden corpus, integration, contract,
  performance, chaos, security scans and the upgrade drill join it as
  their code lands.
- Pull requests enter `develop` through a merge queue, which runs the
  fast suite on each pull request on top of the ones ahead of it. With
  many contributors merging automatically, this stops them racing and
  stops two green pull requests combining into a red `develop`.
- Consequence: the brief's "e2e and axe on every PR" becomes "on every PR
  into `main` and nightly". A regression can sit on `develop` for at most
  one night before the nightly run reports it.

### D32: licence and headers

- The licence is **AGPL-3.0-only** (the brief's "AGPL-3.0"), held as
  `LICENSE` and `LICENSES/AGPL-3.0-only.txt`.
- Every file carries an SPDX header; files that cannot hold a comment are
  covered by `REUSE.toml`. Copyright is held by "The Vellum contributors",
  each of whom signs the DCO.

### D33: the Rust pin, crate layering and merging

- Rust is pinned in `rust-toolchain.toml` (1.97.0).
- Every crate declares a layer in `[package.metadata.vellum]`:
  kernel, contracts, foundation, engine, service, edge, app. A crate may
  depend only on strictly lower layers, so services can never reach into
  each other. `cargo xtask boundaries` enforces this in CI.
- Signed commits will be enforced by branch protection, not CI, because
  CI cannot verify a signature against GitHub's key registry. The rule
  stays off until every contributor, automated ones included, can sign.
- Pull requests need no approval while there is a single maintainer, who
  cannot approve pull requests opened under their own account. Pull
  requests merge automatically once their gate is green.
- One pull request per feature. This supersedes the brief's "PRs under 400
  lines" rule: there is no size gate, and commits inside a pull request
  stay small and logical instead.

### D112: web toolchain pins

- TypeScript stays on 6.0 and ESLint on 9 until `typescript-eslint` and
  `eslint-plugin-react` support the next major versions; Dependabot
  proposes the upgrades.
- This was first recorded under D33 and renumbered so that each decision
  has one number.

### D119: every change is traceable

- Every pull request, except releases, Dependabot, `chore` and `ci`,
  links a GitHub issue that holds the discussion behind it.
- Every feature also adds or updates a decision record in `docs/adr/`
  that links the issue, from `docs/adr/0000-template.md`, or names the
  existing record it follows.
- The fast suite enforces both, so the rule needs no extra required check.
  The product owner asked for it on 2026-10-29 (#157), so that a feature's history
  and the reasoning behind it can be found from the code.

## Gate inventory

| Gate | Suite | Tool |
| --- | --- | --- |
| Rust format, lints, docs | fast | rustfmt, clippy `-D warnings`, rustdoc `-D warnings` |
| Rust tests | fast | cargo-nextest |
| Rust coverage ≥ 85 % | full | cargo-llvm-cov |
| Crate boundaries | fast | `cargo xtask boundaries` |
| Licences, bans, advisories | fast | cargo-deny, cargo-audit, pnpm audit |
| TypeScript | fast | tsc strict, ESLint strictTypeChecked, Prettier, knip |
| Web tests, coverage ≥ 85 % | fast | Vitest |
| End-to-end and accessibility | full | Playwright, axe |
| Files and docs | fast | REUSE, yamllint, shellcheck, markdownlint, Vale, lychee |
| Secrets | fast | gitleaks |
| Infrastructure | fast | sqlfluff, hadolint, helm lint, kube-score (when files exist) |
| Commits and descriptions | fast | Conventional Commits, DCO, branch names, description template |
| Traceability | fast | linked issue; decision record for features |

## Observability baseline

`vellum-kernel::telemetry` is in place from the first commit: JSON logs on
standard error, `tracing` spans, and OTLP export of traces, metrics and logs
when `OTEL_EXPORTER_OTLP_ENDPOINT` is set. The `vellum` binary starts it
before anything else.
