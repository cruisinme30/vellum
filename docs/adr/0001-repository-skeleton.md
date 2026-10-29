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
- Issue: #157
- Pull requests: #2, #36, #183

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
- Pull requests need no approval while there is a single maintainer.
  They merge automatically once their gate is green.
- One pull request per feature, with small logical commits inside it.
  This replaces the brief's "PRs under 400 lines" rule.

### D112: web toolchain pins

- TypeScript stays on 6.0 and ESLint on 9 until `typescript-eslint` and
  `eslint-plugin-react` support the next major versions; Dependabot
  proposes the upgrades.

### D119: every change is traceable

- Every pull request, except releases, Dependabot, `chore` and `ci`,
  links a GitHub issue that holds the discussion behind it.
- Every feature also adds or updates a decision record in `docs/adr/`
  that links the issue, or names the existing record it follows.
- Decision records follow `docs/adr/0000-template.md`: Context,
  Decisions, Alternatives and Consequences, at most 150 lines.
- The fast suite checks all three, so the rule needs no extra required
  check.

## Alternatives

- **A merge queue on `develop`**: needs an organisation-owned
  repository; auto-merge plus the Develop health issue does the job.
- **Release straight from `develop`**: every merge would cancel the
  release's full suite, so releases use frozen snapshot branches.
- **Signed commits now**: CI cannot verify signatures against GitHub's
  key registry, and not every contributor can sign yet.

## Consequences

- The brief's "e2e and axe on every PR" becomes "on every PR into `main`
  and nightly", so a regression can sit on `develop` for one night.
- Two green pull requests can still combine into a red `develop`; the
  Develop health issue blocks merges until it is fixed.
- The checks and the tools behind them are listed in `CONTRIBUTING.md`.
