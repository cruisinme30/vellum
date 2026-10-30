# syntax=docker/dockerfile:1.28
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# The Vellum image: one static binary on distroless (D94; size budget in xtask/src/budgets.rs).
# Build from the repository root:
#
#   docker build -f deploy/docker/Dockerfile -t vellum .
#
# Every base image is pinned by digest and the build is reproducible: two
# builds of the same commit produce byte-identical binaries (D96). The
# release workflow builds the binaries once per architecture and passes them
# in with `--build-arg DIST=release --build-context release=<dir>`, so the
# image ships exactly the binaries that were signed.

# Debian, not Alpine: the Alpine compiler is itself a musl program, and
# musl's allocator made the release build take twice as long. This image's
# compiler links the same static musl binary as a cross target (D271).
ARG RUST_IMAGE=rust:1.97.0-slim-bookworm@sha256:6d220bf85c74e842a79da63997af8d2e74455c0b8847d8bb3a5888572334991d
ARG NODE_IMAGE=node:22-alpine@sha256:fe4f6d74e20370acb04ad889077d14d0f64a9c8ea6a42bc60cddf1e844699914
# Where the binaries come from: "built" (this file) or a named build context.
ARG DIST=built
ARG RUNTIME_IMAGE=gcr.io/distroless/static-debian12:nonroot@sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab

# The web app, built once and embedded into the binary.
FROM ${NODE_IMAGE} AS web
WORKDIR /src/web
ENV CI=true COREPACK_ENABLE_DOWNLOAD_PROMPT=0
COPY web/package.json web/pnpm-lock.yaml ./
RUN corepack enable && pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm run build

# The static musl binary.
FROM ${RUST_IMAGE} AS build
# The build steps pipe into tr, so a failed command must fail the step.
SHELL ["/bin/bash", "-o", "pipefail", "-c"]
ARG TARGETARCH
WORKDIR /src
# musl-gcc compiles the C dependencies for the musl target. SQLCipher in
# vellum-db builds a vendored OpenSSL, whose build runs Perl and make.
# binutils provides readelf for the static-link check below. Not
# pinned: Debian drops superseded package revisions from its mirrors, so an
# exact pin breaks the build at the next security update. They are build
# tools only and never reach the image, and the Rust base image itself is
# pinned by digest.
# hadolint ignore=DL3008
RUN apt-get update \
    && apt-get install -y --no-install-recommends musl-tools perl make binutils \
    && rm -rf /var/lib/apt/lists/*
# Use the image's toolchain (the same version as rust-toolchain.toml)
# without downloading the extra components the file asks for. The musl
# standard library is the one component added, at the same version.
ENV RUSTUP_TOOLCHAIN=1.97.0 \
    CARGO_INCREMENTAL=0 \
    CARGO_TERM_COLOR=never \
    RUSTFLAGS="--remap-path-prefix=/src=. --remap-path-prefix=/usr/local/cargo=cargo" \
    SOURCE_DATE_EPOCH=0
RUN case "${TARGETARCH}" in \
      amd64) triple=x86_64-unknown-linux-musl ;; \
      arm64) triple=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported architecture ${TARGETARCH}" >&2; exit 1 ;; \
    esac \
    && echo "${triple}" > /target \
    && rustup target add "${triple}"
COPY . .
COPY --from=web /src/web/dist web/dist
# musl-gcc is the C compiler only, never the linker. rustc links with the
# host gcc and its own self-contained musl start files and libc. Debian's
# musl-gcc.specs replaces gcc's link rules with an unconditional
# `-dynamic-linker /lib/ld-musl-*.so.1`, so the `-static-pie` link rustc
# uses on x86_64 came out as a PIE with an ELF interpreter that the
# distroless image does not have (D271).
#
# The check after the build fails it if the binary has an ELF interpreter
# or a shared-library dependency: the runtime image has no loader and no
# libraries, so such a binary cannot start ("no such file or directory").
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    triple="$(cat /target)" \
    && export "CC_$(echo "${triple}" | tr - _)=musl-gcc" \
    && cargo build --release --locked --package vellum-cli --target "${triple}" \
    && mkdir -p /out \
    && cp "target/${triple}/release/vellum" /out/vellum \
    && headers="$(readelf --program-headers --dynamic --wide /out/vellum)" \
    && if grep -E 'INTERP|\(NEEDED\)' <<<"${headers}" >&2; then \
         echo "error: the vellum binary is dynamically linked; the runtime image needs a static binary" >&2; \
         exit 1; \
       fi

# The binary, named by architecture.
FROM scratch AS built
ARG TARGETARCH
COPY --from=build /out/vellum /vellum-linux-${TARGETARCH}

FROM ${DIST} AS dist

# Exports just the binary: docker build --target binary --output type=local,dest=out .
FROM scratch AS binary
ARG TARGETARCH
COPY --from=dist /vellum-linux-${TARGETARCH} /vellum

FROM ${RUNTIME_IMAGE} AS base

FROM base AS runtime
ARG TARGETARCH
ARG VERSION=0.0.0
ARG REVISION=unknown
LABEL org.opencontainers.image.title="Vellum" \
      org.opencontainers.image.description="Self-hosted workspace where every object is a Markdown file" \
      org.opencontainers.image.source="https://github.com/cruisinme30/vellum" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"
COPY --from=dist --chmod=0555 /vellum-linux-${TARGETARCH} /usr/local/bin/vellum
# distroless "nonroot" is 65532:65532; the data folder is a volume that
# inherits this ownership.
COPY --from=base --chown=65532:65532 /home/nonroot /data
USER 65532:65532
ENV VELLUM_BIND=0.0.0.0:7700 \
    VELLUM_DATA_DIR=/data \
    VELLUM_PROFILE=single
WORKDIR /data
VOLUME ["/data"]
EXPOSE 7700
ENTRYPOINT ["/usr/local/bin/vellum"]
CMD ["serve"]
