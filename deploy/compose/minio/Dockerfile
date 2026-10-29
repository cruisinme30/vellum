# syntax=docker/dockerfile:1.28
# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only
#
# MinIO server and client, built from pinned upstream source. MinIO stopped
# publishing community images in October 2025, so the compose rung builds
# the last community release itself instead of running an unpatched image
# (ADR 0022, D94). Any S3-compatible endpoint can replace it.

ARG GO_IMAGE=golang:1.25-alpine3.22@sha256:65b4400aee0927412e9ed791a11893273a49d55df24841f7599660fb80dae464
# Alpine rather than distroless: the bucket set-up step needs a shell.
ARG RUNTIME_IMAGE=alpine:3.22@sha256:2e0b81d60597f449baa56dc25d355367869d2dca4cd57e866f1e2b52af3eb55f

FROM ${GO_IMAGE} AS build
ENV CGO_ENABLED=0 GOFLAGS=-trimpath
# RELEASE.2025-10-15T17-29-55Z and RELEASE.2025-08-13T08-35-41Z, by commit.
ADD https://github.com/minio/minio.git#9e49d5e7a648f00e26f2246f4dc28e6b07f8c84a /src/minio
ADD https://github.com/minio/mc.git#7394ce0dd2a80935aded936b09fa12cbb3cb8096 /src/mc
WORKDIR /src/minio
RUN --mount=type=cache,target=/go/pkg/mod go build -ldflags "-s -w" -o /out/minio .
WORKDIR /src/mc
RUN --mount=type=cache,target=/go/pkg/mod go build -ldflags "-s -w" -o /out/mc .

FROM ${RUNTIME_IMAGE}
COPY --from=build /out/minio /out/mc /usr/local/bin/
# Named volumes inherit this ownership, so the server can write as non-root.
RUN mkdir -p /data && chown 65532:65532 /data
USER 65532:65532
ENV MC_CONFIG_DIR=/tmp/mc
EXPOSE 9000 9001
ENTRYPOINT ["/usr/local/bin/minio"]
CMD ["server", "/data", "--console-address", ":9001"]
