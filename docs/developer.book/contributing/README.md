---
id: 01M63CFE66526T2MPAZ2NY485F
title: Contributing
vellum: 1
---

# Contributing

Vellum is open source under AGPL-3.0-only. Contributions are welcome
from anyone who signs off their commits under the Developer Certificate
of Origin.

## The workflow {#workflow}

```mermaid
gitGraph
  commit id: "release 1.0"
  branch develop
  checkout develop
  commit id: "kernel"
  branch feat/my-change
  checkout feat/my-change
  commit id: "feat: add the change"
  commit id: "test: cover it"
  checkout develop
  merge feat/my-change
  checkout main
  merge develop tag: "v1.1.0"
```

::: steps

1. **Branch** from `develop` as `<type>/<topic>`, such as `feat/typed-ids` or
   `docs/help-books`.
2. **Commit** with Conventional Commits, a body that says why, and
   `git commit -s` for the sign-off.
3. **Open a pull request** into `develop` that fills in every section of the
   template.
4. **Pass the fast suite**: formatting, linters, tests, crate boundaries, supply
   chain, commit rules and labels.
5. **Merge** with a squash once the suite is green.

:::

`develop` merges into `main` when the full suite passes: coverage,
release build, end-to-end and accessibility tests, and external links.
Releases are tagged on `main`.

From a pushed branch to a release:

```mermaid
flowchart LR
  B[feat/... branch, signed and signed off] --> I[Integration batch]
  I --> D[develop: fast suite]
  D --> N[Nightly and chaos runs]
  D --> R[Release pull request: full suite, CodeQL]
  R --> M[main: one signed snapshot]
  M --> T[Tag vX.Y.Z: signed binaries, images and the website]
```

| Stage | What runs | Blocks a release when red |
| --- | --- | --- |
| Pull request into `develop` | Formatting, lints, tests, boundaries, commit rules | Yes |
| `develop` | The fast suite again on the merged result | Yes |
| Nightly | Fuzzing, load, chaos and the browser matrix | Yes |
| Release pull request | The full suite, coverage, CodeQL, external links | Yes |

## One feature per pull request {#one-feature}

A pull request carries one feature, complete with its tests and
documentation, as several commits if that reads better. Do not split a
feature into stacked pull requests.

## Set up {#setup}

```bash
git clone https://github.com/cruisinme30/vellum && cd vellum
git config core.hooksPath .githooks
git config commit.template .gitmessage
cargo nextest run --workspace
cd web && pnpm install && pnpm test
```

`vellum serve` embeds the web app from `web/dist` when the binary is
compiled. Run `pnpm --dir web build` before `cargo build` to include it;
without it, the binary still builds and its home page says the web app
was not built.

## Decisions {#decisions}

Decisions are recorded as architecture decision records in `docs/adr/`,
each with numbered decisions (D31 onwards). Changing a settled decision
needs a new decision that says why.
