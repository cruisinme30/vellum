<!--
SPDX-FileCopyrightText: 2026 The Vellum contributors
SPDX-License-Identifier: AGPL-3.0-only
-->

# Contributing to Vellum

## Branches

```mermaid
gitGraph
  commit id: "init"
  branch develop
  checkout develop
  branch feat/a
  commit id: "feat a"
  checkout develop
  merge feat/a id: "fast suite"
  branch fix/b
  commit id: "fix b"
  checkout develop
  merge fix/b id: "fast suite "
  checkout main
  merge develop id: "full suite" tag: "v1.1.0"
```

| Branch | Purpose | Merges in from | Gate |
| --- | --- | --- | --- |
| `develop` | Integration branch; all work starts here | short-lived feature branches | **Fast suite** |
| `main` | Always releasable; releases are tagged here | `develop` only, at regular intervals | **Full suite** |
| `<type>/<topic>` | One change each | branched from `develop` | — |

1. Branch from `develop` with a name of the form `<type>/<topic>`, where
   type is a Conventional Commits type and topic says what changes, in
   kebab case: `feat/typed-ids`, `fix/watcher-rename`, `docs/adr-0002`.
   CI rejects other names (Dependabot's `dependabot/…` excepted). Open the
   pull request against `develop`.
2. Merge through the merge queue: `gh pr merge --auto --squash`, or
   **Merge when ready** on GitHub. The queue tests each pull request on
   top of the ones ahead of it and merges it once **Fast suite** passes
   there, so parallel pull requests never race each other into `develop`.
   If `Cargo.lock` or `web/pnpm-lock.yaml` conflicts, merge `develop` into
   your branch and run `cargo xtask relock`.
3. At regular intervals a maintainer opens a pull request from `develop`
   into `main`. It merges with a merge commit, never a squash, so history
   stays shared.
4. Releases are cut by tagging `main` (`vX.Y.Z`, semver).
5. An urgent fix branches from `main`, merges into `main` through the full
   suite, then `main` merges back into `develop`.

## Checks

| Suite | Runs on | Contents |
| --- | --- | --- |
| Fast (`ci.yml`) | every pull request into `develop` or `main`, each merge queue entry, pushes to `develop` | rustfmt, clippy, rustdoc, unit and property tests (nextest), crate boundaries, cargo-deny, cargo-audit; Prettier, tsc, ESLint, knip, Vitest with 85 % coverage, pnpm audit; REUSE, yamllint, shellcheck, markdownlint, Vale, local links, gitleaks, infrastructure linters; Conventional Commits, DCO, branch name and description template; linked issue and decision record |
| Full (`full.yml`) | every pull request into `main`, pushes to `main`, nightly | the fast suite, plus Rust line coverage of at least 85 %, the release build, Playwright end-to-end with axe accessibility checks, and external link checks |

Further layers join the full suite as their code lands: golden corpus,
integration with testcontainers, contract, performance, chaos, security
scans and the upgrade and restore drill.

Run the fast suite locally before pushing. Install the hooks once, after
cloning:

```sh
cargo xtask install-hooks
```

This turns on the commit template and three git hooks: `commit-msg`
checks the commit message, `pre-commit` checks formatting, and `pre-push`
runs `cargo xtask fast`, which mirrors every **Fast suite** check in CI
and refuses the push if one fails. Run it by hand at any time:

```sh
cargo xtask fast
```

A check whose tool is not installed is skipped and named in the summary;
CI still runs it. Install `markdownlint-cli2`, `vale`, `lychee`,
`gitleaks`, `reuse`, `yamllint` and `shellcheck` to run every check. In
an emergency, `git push --no-verify` skips the hook, but CI still gates
the merge.

## Issues and decision records

Every change is traceable to the discussion that led to it.

- **One issue per feature or fix.** Open a GitHub issue before starting.
  Keep the discussion there: the problem, the options, the questions and
  the answers, so the issue is the feature's full history.
- **The pull request links it.** Put `Closes #N` (or `Refs #N` when the
  issue stays open) in the **Issue** section of the description.
- **A feature records its decisions.** Copy `docs/adr/0000-template.md`
  to `docs/adr/NNNN-<slug>.md`, ask for the next record and decision
  numbers, and fill it in: context, options, decisions, consequences, and
  a link to the issue. When a feature only extends an existing decision,
  update that record or name it as `ADR NNNN` in the description.

The **Issue and decision record** check in the fast suite enforces this.
Release pull requests, Dependabot, and `chore` or `ci` pull requests are
exempt.

## Commits and pull requests

CI enforces every rule in this section on each pull request.

- **Commit template**: `cargo xtask install-hooks` sets it up. Each
  commit has a Conventional Commits subject, `type(scope): summary` of at
  most 72 characters, where type is one of
  `build chore ci docs feat fix perf refactor revert style test`; then a
  blank line; then a body saying why. The `.githooks/commit-msg` hook
  checks the same rules locally.
- **Pull request title**: a Conventional Commits subject, because it
  becomes the squash commit on `develop`.
- **Pull request description**: the template in
  `.github/pull_request_template.md`. Summary, Requirements, Changes,
  Tests, Observability and Security must each be filled in; write "None"
  with a reason when one does not apply.
- **Developer Certificate of Origin**: sign off every commit with
  `git commit -s`. The sign-off must match the commit author.
- **Signed commits** (planned): sign with GPG, SSH or S/MIME so GitHub
  marks them verified. Branch protection will require this once every
  contributor, including automated ones, has a signing key set up.
- **Scope**: one pull request per feature, complete with its tests and
  docs, not one per commit or step. There is no line limit; keep commits
  inside the pull request small and logical so the history reads well.

## Labels

Labels are code: `.github/labels.yml` defines them and the Sync labels
workflow applies changes when that file lands on `develop`.

| Group | Labels | Set by |
| --- | --- | --- |
| `type/*` | `feat fix docs refactor perf test build ci chore style revert` | the pull request title |
| `area/*` | `kernel contracts format store crypto db render export authz collab collections pages work my-work api jobs mcp cli editor web ci docs deploy` | changed paths (`.github/labeler.yml`), or by hand |
| `phase/*` | `foundation format editor drive-books collections work mywork-mcp clients-release` | by hand, following the build order |
| `status/*` | `in-progress blocked ready-to-merge` | by hand |
| `priority/p0` | every requirement is P0 | by hand |
| `size/*` | `xs s m l xl` (under 50, 200, 500, 1,000 lines, or more) | changed lines, generated files excluded |
| `release` | a `develop` → `main` pull request | automatically |

The **Labels** check fails a pull request without at least one `type/*`
and one `area/*` label. Branch protection requires it next to the suite
check. When a new crate lands, add its path to `.github/labeler.yml`.

## Definition of done

Formatted; zero lint warnings; tests at the right layer; tracing spans and
metrics for any new I/O; public items documented; user guide updated if the
change is visible; a linked issue, and for a feature an ADR in
`docs/adr/` that links it; reviewed
by someone who did not write it; CI green.

## Branch protection

Set these under **Settings → Rules → Rulesets** on GitHub.

| Setting | `develop` | `main` |
| --- | --- | --- |
| Require a pull request (0 approvals while there is one maintainer) | yes | yes |
| Required status checks | `Fast suite` | `Full suite` |
| Merge queue (squash, groups of up to 5) | yes | no |
| Require branches to be up to date | no (the merge queue does it) | yes |
| Require signed commits | planned | planned |
| Require linear history | yes (squash merge) | no (merge commits from `develop`) |
| Block force pushes and deletion | yes | yes |
| Restrict who can push | — | maintainers only, and only from `develop` or a hotfix |
