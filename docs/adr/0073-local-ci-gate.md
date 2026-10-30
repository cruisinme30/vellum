---
id: 01M65PWJV9WC94RE8V4P8K7XWE
title: "ADR 0073: A local CI gate before develop, main and releases"
status: draft
owner: "@cruisinme30"
tags: [adr, ci, tooling]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D318
- Issue: #532
- Pull requests: to be linked when merged

:::

## Context

On the night of 2026-10-29 a release was rebuilt four times, because each
CI run found one more failure that a local run would have caught. The
local checks ([ADR 0048](0048-catch-ci-failures-before-pushing.md)) are
focused on what a branch changed, skip the browsers and the release build,
and need Linux tools that a Mac does not have; native Rust builds on macOS
are unreliable. The hooks that run them were also never turned on there.
Every check CI runs has to pass on the machine before a push to develop,
main or a release branch.

## Decisions

### `D318`: `scripts/dev/gate.sh` gates pushes to develop, main and releases

- One command runs, on the committed HEAD tree, what CI runs: `cargo xtask
  fast` with every area and test (Postgres and Meilisearch included), the
  website build and check, and the whole Playwright suite in Chromium,
  Firefox and WebKit (`UI_SCOPE=full`, visual baselines in Chromium) in
  the pinned Playwright image. `--release` adds what release pull
  requests run: Rust coverage of at least 85 %, two byte-identical amd64
  release builds with the image and `cargo xtask budgets`, ClamAV,
  external links, the nightly results and CodeQL.
- The checks run in Linux containers, natively arm64 on Apple silicon:
  `vellum-ci-local` (`scripts/dev/ci-image/Dockerfile`) takes every
  version from where CI pins it and checks every download's SHA-256.
  `scripts/dev/install-tools.sh` now covers Linux aarch64 as well.
- The gate checks out the commit into a Docker volume, so it tests the
  commit and not uncommitted edits, and writes nothing into the checkout.
  Stages run side by side with their own logs, every container is
  labelled and removed on exit or Ctrl-C, and caches stay in named volumes.
- When every stage passes, it stamps the tree hash under the repository's
  git directory. A tree, not a commit, so rewording a message keeps it.
- The pre-push hook refuses a push to develop without a stamp for the
  pushed tree, and to main or `chore/release-*` without a `--release`
  stamp, and prints the command to run. Other branches still run the
  focused Fast suite, in the gate's container when cargo is missing.
- `scripts/dev/push.sh <branch> [<commit>]` is the one way to push, and
  the only one Claude sessions may use: it refuses without the stamp, and
  for develop while a Fast suite run is in progress there (unless
  `--supersede`); release branches are replaced with `--force-with-lease`
  against their current SHA. The pre-commit hook checks only staged files.

## Alternatives

- **Rely on the pre-push Fast suite**: it is focused and leaves out the
  browsers and the release build, which is how the four rebuilds happened.
- **Run the checks natively on the Mac**: Rust builds there are unreliable
  and several linters have no macOS build at CI's version.
- **Test the working tree in place**: the stamp would then vouch for edits
  that are not in the commit, and the run would write into the checkout.
- **Stamp the commit hash**: amending a message would force a rerun of a
  tree that already passed.

## Consequences

- A push to develop or a release costs a local run (tens of minutes cold,
  less with warm caches), but CI should then pass the first time.
- The gate runs CI's checks as they are: nothing is skipped, retried or
  allow-listed to get a green run. What it cannot check is printed at the
  end of every run: the pull request's title, description and linked
  issue (`cargo xtask pr-check` covers them before opening it), GitHub's
  dependency review, and zizmor's online audits unless `GH_TOKEN` is set.
- **CodeQL does not scan Rust locally**: the macOS CodeQL CLI cannot
  extract it, so CI's CodeQL run stays the only Rust scan.
- The x86 stage runs under emulation and is slow, so it waits for the
  other stages on a small Docker VM; main and release branches need it.
- Each browser runs its whole suite in one container, not CI's four
  shards: the same tests, more slowly.
- `git push --no-verify` still skips the hooks; CI remains the backstop.
