# syntax=docker/dockerfile:1.28
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# The Vellum image: one static binary on distroless, under 40 MB (D94).
# Build from the repository root:
#
#   docker build -f deploy/docker/Dockerfile -t vellum .
#
# Every base image is pinned by digest and the build is reproducible: two
# builds of the same commit produce byte-identical binaries (D96). The
# release workflow builds the binaries once per architecture and passes them
# in with `--build-arg DIST=release --build-context release=<dir>`, so the
# image ships exactly the binaries that were signed.

ARG RUST_IMAGE=rust:1.97.0-alpine3.22@sha256:3ab54ebc56cb9b6544edfbbb9f171d1576ecf86f5a0a37a231b0e32ff3b48e0c
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
WORKDIR /src
# Use the image's toolchain (the same version as rust-toolchain.toml)
# without downloading the extra components the file asks for.
ENV RUSTUP_TOOLCHAIN=1.97.0 \
    CARGO_INCREMENTAL=0 \
    CARGO_TERM_COLOR=never \
    RUSTFLAGS="--remap-path-prefix=/src=. --remap-path-prefix=/usr/local/cargo=cargo" \
    SOURCE_DATE_EPOCH=0
COPY . .
COPY --from=web /src/web/dist web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    cargo build --release --locked --package vellum-cli \
    && mkdir -p /out \
    && cp target/release/vellum /out/vellum

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
