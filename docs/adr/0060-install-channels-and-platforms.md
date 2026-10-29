---
id: 01M64TJMHX1AVRWE81QQVZ03HC
title: "ADR 0060: Install channels, platforms and signed desktop builds"
status: draft
owner: "@cruisinme30"
tags: [adr, deploy, release, desktop]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D250, D251, D252
- Issue: #188, #187, #184
- Pull requests: the integration batch that carries `feat/install-channels`

:::

## Context

The brief's lightweight promise (R-LT-1, 2, 3, 5) is that Vellum installs
with `brew install`, `winget`, `apt`/`rpm` or a static Linux binary, runs
with `vellum serve` and no flags, works on x86 and ARM across macOS, Linux
and Windows including Raspberry Pi-class hardware, and stays under 80 MB
and 1 % CPU when idle. [ADR 0022](0022-deployment-and-release.md) (D96)
already builds reproducible, Sigstore-signed static Linux binaries; this
record extends that release to every channel and to the desktop app.
Discussion is on [#188](https://github.com/cruisinme30/vellum/issues/188),
[#187](https://github.com/cruisinme30/vellum/issues/187) and
[#184](https://github.com/cruisinme30/vellum/issues/184).

## Decisions

### D250: every channel installs a signed release file

- The release carries static Linux binaries (amd64, arm64), a universal
  macOS binary, Windows x64 and arm64 portable binaries, and `.deb` and
  `.rpm` packages built by nfpm from `deploy/packages`. All are in
  SHA256SUMS, signed with Sigstore and covered by SLSA provenance.
- The packages install `/usr/bin/vellum`, a `vellum` system user and a
  hardened systemd unit reading `/etc/vellum/vellum.env`. The unit is
  installed but not enabled, and the data folder survives removal.
- `cargo xtask installers` writes the Homebrew formula (tap
  `cruisinme30/homebrew-vellum`, with `brew services`) and the winget
  manifests (`Vellum.Vellum`, portable) from SHA256SUMS, so they can only
  point at signed files. Both are attached to the release.
- The release pushes the tap and opens the winget pull request when their
  tokens exist, and skips each with a notice when they do not.
- `site/install.sh` (`curl … | sh`) installs the binary on Linux and
  macOS after checking its checksum, and its signature when cosign is
  present. `vellum serve` needs no flags: data in `./vellum-data`.

### D251: 64-bit platforms, with the idle budgets measured

- Supported: x86-64 and ARM64 on Linux, macOS and Windows. The Raspberry
  Pi target is a Pi 3 or later on a 64-bit OS.
- 32-bit ARM is not built: it would be a second musl and vendored-OpenSSL
  toolchain for hardware the 64-bit OS already covers.
- `cargo xtask budgets` samples CPU time over 5 s once the server is idle
  and fails at 1 % of one core, next to the size, cold-start, idle-memory
  and drain budgets (R-LT-1).
- Lazy subsystems (R-LT-2) are held to by those idle budgets: the tantivy
  writer lives for one batch, Mermaid renders in-process per request, and
  export runs per request with nothing resident between requests.

### D252: the desktop app bundles the released binaries; Azure signing

- The desktop jobs no longer compile the server. They bundle the release
  binaries, one per architecture (`target/desktop-bin/<arch>/`), so the
  arm64 Linux and Windows apps stop shipping an x64 server.
- Linux gains an rpm next to the AppImage and deb. The macOS merge keeps
  the already universal server binary as is (`x64ArchFiles`).
- Windows signing uses Azure Trusted Signing for the CLI binaries and the
  app. Since June 2023 code-signing keys must live in hardware, so a
  `.p12` in a CI secret is no longer an option.
- Tags before 1.0 may ship an unsigned Windows build with a notice. From
  1.0 the release fails without Windows signing, as it already does
  without macOS notarisation (ADR 0023, D99).

## Alternatives

- **homebrew-core**: needs notability and builds from source; our own tap
  ships now, and a core formula can follow.
- **A hosted apt/yum repository**: needs a GPG key and hosting; packages
  on the release install with `apt install ./vellum_….deb` today.
- **cargo-dist or GoReleaser**: would replace the reproducible, signed
  pipeline ADR 0022 already has instead of extending it.
- **An EV or OV certificate file for Windows**: no longer issued for CI
  use; a cloud HSM costs more than Azure Trusted Signing.

## Consequences

- Publishing waits on accounts only the owner can create: the Azure
  Trusted Signing account, the tap repository and token, and the winget
  token. Until then the files are on the release and each step says so.
- The first winget submission is reviewed by Microsoft's moderators.
- The native Windows arm64 binary is cross-compiled and not run in CI; a
  Windows arm64 runner can add that smoke test later.
- `deploy/packages/nfpm.yaml` and `xtask/src/installers.rs` hold the
  release file names; renaming a release file means changing both.
