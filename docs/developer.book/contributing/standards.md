---
id: 01M63CFE669ES7E5ECEZ0VCM61
title: Engineering standards
vellum: 1
---

# Engineering standards

CI enforces every rule here; warnings are errors.

## Rust {#rust}

::: facts

- Forbidden: `unsafe` code
- Denied: `unwrap`, `expect`, `panic!`, indexing that can panic, `todo!`, `dbg!` and `println!`
- `clippy`: all lints denied; pedantic and nursery as warnings, which CI treats as errors
- Documentation: every public item documented; `cargo doc` with warnings as errors
- Tests: `cargo nextest`; line coverage of at least 85% on core crates

:::

## TypeScript {#typescript}

::: facts

- Compiler: strict, `noUncheckedIndexedAccess`, `exactOptionalPropertyTypes`, `noImplicitOverride`, `verbatimModuleSyntax`
- Lint: typescript-eslint strict type-checked and stylistic; React, hooks, accessibility, imports, unicorn and sonarjs rules
- Limits: 400 lines per file, cyclomatic complexity 12, no default exports
- Tests: Vitest with at least 85% coverage

:::

## Everything else {#other}

Formatting with rustfmt and Prettier; shellcheck, yamllint, hadolint,
Helm lint and kube-score; markdownlint and Vale for prose; a link
checker; gitleaks for secrets; REUSE headers on every file; Conventional
Commits with a sign-off.

## Tests by layer {#tests}

| Layer | What | When |
| --- | --- | --- |
| Unit and property | Parser, CRDT, authorization, formulas | Every pull request |
| Golden corpus | Every block through every stage | Every pull request |
| Integration | Real databases and storage with testcontainers; migrations | Every pull request |
| Contract | OpenAPI validation, generated client, protocol fuzzing | Every pull request |
| End-to-end | Playwright, two-browser collaboration, axe | Into `main` and nightly |
| Performance | k6; a 10% p95 regression fails | Nightly |
| Chaos | Killed replicas, storage outages, network faults | Weekly |
| Security | Audits, ZAP, penetration test | Weekly and before releases |
| Upgrade and restore | From the previous release, restore drill | Every release |

## Observability {#observability}

Any new I/O gets a tracing span and metrics through
`vellum_kernel::telemetry`. Spans name the operation and carry the
space, the object and sizes, never content or secrets.
