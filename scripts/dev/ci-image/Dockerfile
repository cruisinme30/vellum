# syntax=docker/dockerfile:1
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# vellum-ci-local: every Fast suite tool at CI's pinned versions, for the
# local gate (scripts/dev/gate.sh, ADR 0073). Native arm64 on Apple silicon;
# the gate's opt-in x86 stage builds it for linux/amd64 too. The versions
# come from the repository, where CI pins them:
#   - Rust and its components from rust-toolchain.toml,
#   - the Python linters from .github/requirements/hygiene.txt, by hash,
#   - the release binaries from scripts/dev/install-tools.sh and
#     scripts/ci/install-infra-tools.sh, each checked against its SHA-256,
#   - pnpm from web/package.json's packageManager, through corepack.
# gate.sh builds it from `git archive` of just those files, tagged with a
# hash of them, so changing a pin rebuilds the image:
#
#   git archive HEAD rust-toolchain.toml .github/requirements/hygiene.txt \
#     scripts/dev scripts/ci/install-infra-tools.sh web/package.json |
#     docker build -f scripts/dev/ci-image/Dockerfile -t vellum-ci-local -

ARG UBUNTU=ubuntu:24.04@sha256:fe81560990fae86dc4743202b2cb4d3df98d0bcf949173cecef15ce1127ef852
FROM ${UBUNTU}
ARG TARGETARCH
SHELL ["/bin/bash", "-o", "pipefail", "-c"]

ENV DEBIAN_FRONTEND=noninteractive \
    RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    COREPACK_HOME=/usr/local/share/corepack \
    COREPACK_ENABLE_DOWNLOAD_PROMPT=0 \
    UV_PYTHON_INSTALL_DIR=/usr/local/share/uv-python \
    PATH=/usr/local/cargo/bin:/usr/local/node/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

# Build tools for the workspace, git for the checks that read history, and
# the Docker CLI with Compose for testcontainers and the Compose lint (the
# daemon is the host's, through its socket). Ubuntu signs these packages.
# hadolint ignore=DL3008
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       build-essential pkg-config libssl-dev perl clang lld curl git tzdata \
       ca-certificates cmake protobuf-compiler xz-utils python3 python3-venv \
       docker.io docker-compose-v2 \
    && rm -rf /var/lib/apt/lists/*

# rustup-init, checked against the SHA-256 static.rust-lang.org publishes,
# then exactly the toolchain and components rust-toolchain.toml names.
ARG RUSTUP=1.29.2
WORKDIR /tmp/build
COPY rust-toolchain.toml ./
RUN case "${TARGETARCH}" in \
      arm64) triple=aarch64-unknown-linux-gnu; sha=a9c168dbea81a84671c6b7a5c1795cb0eb699020110d107ddfca427bd797e697 ;; \
      amd64) triple=x86_64-unknown-linux-gnu; sha=5be191e8c70a5007074254cb74d78dbfdd25cd5a61c27de7b8e5410122ec24f4 ;; \
      *) echo "unsupported architecture ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
    && curl -sSfL --retry 3 -o rustup-init "https://static.rust-lang.org/rustup/archive/${RUSTUP}/${triple}/rustup-init" \
    && echo "${sha}  rustup-init" | sha256sum --check --quiet \
    && chmod +x rustup-init \
    && ./rustup-init -y --no-modify-path --profile minimal \
        --default-toolchain none \
    && rustup toolchain install \
    && rustup default \
        "$(sed -nE 's/^channel = "([^"]+)"/\1/p' rust-toolchain.toml)" \
    && rm rustup-init && rustc --version && cargo clippy --version

# Node 22, as CI's setup-node, checked against nodejs.org's SHASUMS256.
ARG NODE=22.23.4
RUN case "${TARGETARCH}" in \
      arm64) arch=arm64; sha=6287aa46b6c375d816e63ae66d024fbbfd70404825aabdfdf898a970871d2f4b ;; \
      amd64) arch=x64; sha=1805bcd35e6815559bd2eb97db438d3fdb0eb3a5e6a977b7c1badca76337e9ce ;; \
    esac \
    && curl -sSfL --retry 3 -o node.tar.xz "https://nodejs.org/dist/v${NODE}/node-v${NODE}-linux-${arch}.tar.xz" \
    && echo "${sha}  node.tar.xz" | sha256sum --check --quiet \
    && mkdir -p /usr/local/node \
    && tar -xJf node.tar.xz -C /usr/local/node --strip-components=1 \
    && rm node.tar.xz && node --version

# pnpm at web/package.json's packageManager version; corepack checks the
# package's signature.
COPY web/package.json web-package.json
RUN pnpm="$(sed -nE 's/.*"packageManager": "(pnpm@[^"]+)".*/\1/p' \
        web-package.json)" \
    && test -n "${pnpm}" \
    && corepack enable pnpm \
    && corepack install --global "${pnpm}" \
    && pnpm --version

# uv, to give the Python linters CI's Python 3.13.
ARG UV=0.13.6
RUN case "${TARGETARCH}" in \
      arm64) triple=aarch64-unknown-linux-gnu; sha=229e194255c7958d86d56e8790d969a6f74736592133f0dfd0076f754c642c1d ;; \
      amd64) triple=x86_64-unknown-linux-gnu; sha=f29c3e94ffabd7e6f3ba521dba169a9b7ec12100bf3257087f0f24f6123f3645 ;; \
    esac \
    && curl -sSfL --retry 3 -o uv.tar.gz "https://github.com/astral-sh/uv/releases/download/${UV}/uv-${triple}.tar.gz" \
    && echo "${sha}  uv.tar.gz" | sha256sum --check --quiet \
    && tar -xzf uv.tar.gz --strip-components=1 -C /usr/local/bin \
    && rm uv.tar.gz && uv --version

# Every other Fast suite tool, through the same script contributors use.
COPY .github/requirements/hygiene.txt .github/requirements/hygiene.txt
COPY scripts/dev/install-tools.sh scripts/dev/install-tools.sh
COPY scripts/ci/install-infra-tools.sh scripts/ci/install-infra-tools.sh
RUN VELLUM_TOOLS_DIR=/usr/local/bin \
    VELLUM_TOOLS_VENV=/usr/local/share/vellum-hygiene \
    VELLUM_INSTALL_HELM=1 bash scripts/dev/install-tools.sh

WORKDIR /
RUN rm -rf /tmp/build \
    && git config --system --add safe.directory '*'
