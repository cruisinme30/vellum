---
id: 01M64YAVVG4KNZ6GK33K25TRW7
title: "ADR 0066: Build the release binary on a glibc host"
status: draft
owner: "@cruisinme30"
tags: [adr, ci, deploy, foundation]
vellum: 1
---

::: facts

- Date: `2026-10-29`
- Status: accepted
- Decisions: `D271`
- Issue: `#516`
- Pull requests: `ci/faster-release-build`

:::

## Context

The release check builds the static musl binary twice and compares the
two builds. Those builds were its longest path: 27 and 34 minutes in run
40957380701, while every other job finished in under 20. With a release
every six hours, that build sets how fast each release lands, and it
costs about an hour of runner time per release.

On a 4-core sandbox with a glibc host, the same release build takes 17.5
minutes. In CI it ran on `rust:alpine`, where the compiler itself links
against musl. musl's allocator is slow under rustc and LLVM: the final
`vellum-cli` step (code generation, thin LTO and the link) took 12
minutes there, and 6 minutes on the glibc host.

## Decisions

### `D271`: the build stage cross-compiles to musl from Debian

- The Dockerfile's `build` stage uses `rust:1.97.0-slim-bookworm`, pinned
  by digest. It adds the `*-unknown-linux-musl` standard library for the
  same toolchain, and `musl-gcc` from `musl-tools` to compile the C
  dependencies (SQLCipher's vendored OpenSSL, zstd, aws-lc).
- `TARGETARCH` picks the target: `amd64` builds `x86_64-unknown-linux-musl`
  and `arm64` builds `aarch64-unknown-linux-musl`. The output is the same
  static binary, so the stage names, `DIST`, the build context and
  `vellum-linux-<arch>` all stay the same.
- Measured in CI on 2026-10-29 (run 40965159256, three runners in
  parallel):
  - Alpine took 31.9 minutes.
  - Debian took 14.1 and 17.5 minutes.
  - The two Debian builds were byte-identical, both 101,125,488 bytes.

## Alternatives

- **16 codegen units**: 8% faster on its own, but the binary grew 18% (118
  MB against 101 MB), past the size budget.
- **Cache dependencies between runs** (cargo-chef, BuildKit cache): the
  reproducibility check needs at least one build from scratch, so the
  release check would still wait for an uncached build.
- **System OpenSSL instead of the vendored copy**: saves about 4 minutes,
  but ties the binary to whatever revision the distribution mirror has
  that day.
- **A faster allocator for Alpine's rustc**: not something the rust image
  offers, and it would mean patching the toolchain.

## Consequences

- The release check's longest path drops by about 15 minutes. The two
  builds now take about 14 to 18 minutes instead of 27 to 34.
- Reproducibility and SLSA provenance are unchanged: the release workflow
  still builds twice per architecture, compares the builds and attests
  what it built.
- `musl-tools`, Perl and make come from Debian's mirror without a pinned
  version, for the same reason the Alpine packages were not pinned. They
  are build tools only and never reach the image.
- The binary changes by a few hundred kilobytes (101.1 MB against 101.4
  MB), within the budget.
