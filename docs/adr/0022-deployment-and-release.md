---
id: 01M63DXZP6B6T27ZHQ5RGSDTQG
title: "ADR 0022: Deployment ladder, signed releases and load harnesses"
status: verified
owner: "@cruisinme30"
tags: [adr, deploy, release, performance]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D94, D95, D96
- Issue: #189, #190, #193, #194, #196, and #187 for the budgets
- Pull requests: #245

:::

## Context

The brief (sections 2, 11 and 12) sets the deployment ladder (single
binary, `docker run`, `docker compose` with Postgres, MinIO and Kroki,
Helm with two of everything, a Postgres replica and shards by space), a
canary 10/50/100 with automatic rollback, a signed and reproducible
supply chain (CycloneDX SBOM, Sigstore, SLSA level 3), nightly k6 with a
10 % p95 gate, weekly chaos, a ZAP scan, an upgrade and restore drill per
release, and the single-binary budgets: under 40 MB to download, under
2 s cold start, under 80 MB idle. The issues hold the discussion:
[#189](https://github.com/cruisinme30/vellum/issues/189),
[#190](https://github.com/cruisinme30/vellum/issues/190),
[#193](https://github.com/cruisinme30/vellum/issues/193),
[#194](https://github.com/cruisinme30/vellum/issues/194),
[#196](https://github.com/cruisinme30/vellum/issues/196) and
[#187](https://github.com/cruisinme30/vellum/issues/187).

MinIO stopped publishing community images in October 2025, and the binary
has no `serve` command yet, so the harnesses must exist before the server
they test.

## Decisions

### D94: the deployment ladder and its contract

- One image, `deploy/docker/Dockerfile`: a static musl binary on
  distroless `static`, running as UID 65532, every base pinned by digest.
  `docker run` and compose use it.
- `deploy/compose/compose.yaml` is the Team profile: Vellum, Postgres 17,
  MinIO (built from pinned source in `deploy/compose/minio`, bucket
  versioning on) and Kroki with its Excalidraw and draw.io companions. Only
  Vellum publishes a port; containers are read-only with no capabilities.
- `deploy/helm/vellum` is the Organisation profile: at least two replicas
  (an autoscaler cannot go below two), required anti-affinity, a
  disruption budget, network policies (Kroki has no egress), Kroki with two
  replicas, and one Patroni cluster per shard with a synchronous replica
  (`synchronous_mode`, not strict, so the primary keeps writing when the
  replica is down and failover never loses an acknowledged write).
- Shards by space: `shards` lists shard names; each gets its own
  Postgres cluster. The server reads `VELLUM_SHARDS` (ordered, the first
  is the home shard that holds the shard directory) and
  `VELLUM_SHARD_<NAME>_DATABASE_URL`; `VELLUM_DATABASE_URL` is the home
  shard. Shards are added, never renamed.
- The server contract the deploy files rely on: the kernel's
  `VELLUM_PROFILE`, `VELLUM_BIND`, `VELLUM_DATA_DIR`, `VELLUM_PUBLIC_URL`,
  `VELLUM_DATABASE_URL`; plus `VELLUM_BLOB_URL`, `VELLUM_S3_ENDPOINT`,
  `VELLUM_S3_REGION`, `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
  `VELLUM_KROKI_URL`, `VELLUM_CLAMD_ADDRESS`, and `VELLUM_KEK_PASSPHRASE` or
  `VELLUM_KEK_PASSPHRASE_FILE`. `vellum serve` answers `GET /healthz`
  and `GET /readyz`, and on SIGTERM drains and exits 0 within 30 s.

### D95: canary 10/50/100 with automatic rollback

- With `canary.enabled`, the chart renders an Argo Rollouts `Rollout`
  instead of the Deployment: 10 %, pause, 50 %, pause, 100 %, with
  traffic split by the NGINX ingress.
- Background analysis from the first step queries Prometheus for the
  canary pods' server-error ratio (limit 0.1 %) and p95 latency (limit
  150 ms, the uncached-read target). One failed check aborts the rollout
  and traffic returns to the stable version.
- Off by default so the chart installs without the Argo controller;
  `values-production.yaml` turns it on.

### D96: signed releases and the test harnesses

- Releases are tags `vX.Y.Z` on `main` whose version matches
  `Cargo.toml`. `.github/workflows/release.yml` builds each architecture
  twice from scratch and fails unless the binaries are identical, writes a
  CycloneDX SBOM, signs every file with keyless cosign, records SLSA level
  3 provenance for the files and the image, builds the multi-arch image
  from the same binaries, signs it and attests its SBOM, then publishes
  the GitHub release.
- The Full suite builds the binary twice and compares it, and runs
  `cargo xtask budgets`: first ready response under 2 s, idle resident
  memory under 80 MB, and a clean exit within 30 s of SIGTERM. The size
  budgets (binary 110 MB, compressed image 45 MB) sit just above the
  first sizes measured, in release 3, so growth fails; reaching the 40 MB
  download target is
  [#491](https://github.com/cruisinme30/vellum/issues/491).
- Nightly: k6 (`deploy/perf/k6`) against the compose stack with the SLOs as
  thresholds; `cargo xtask perf-gate` fails when any p95 grows 10 % or more
  over `deploy/perf/baseline.json`, and the result is kept as trend data.
  The baseline starts at the SLO targets and is replaced by the first
  measured run through a pull request. ZAP runs its baseline scan on the
  same stack.
- Weekly: `deploy/chaos/run.sh` on kind kills a Vellum node, a Kroki pod
  and the Postgres primary under load; it fails when a threshold breaks or
  an acknowledged write cannot be read back.
- Every release: `deploy/drills/upgrade.sh` upgrades a seeded stack from
  each of the previous two releases, and `deploy/drills/restore.sh`
  restores a backup into a scratch stack; both compare row and object
  counts.
- Until `vellum serve` exists, the server-dependent steps report a notice
  and pass; size budgets and reproducibility are enforced now.

## Alternatives

- **Base image**: distroless `static` with a musl binary (4 MB compressed
  today) was chosen over Alpine (a shell to patch) and `scratch` (no CA
  bundle, no non-root user).
- **MinIO**: building the last community release from pinned source was
  chosen over an old image without security fixes, or over switching the
  brief's choice to another S3 server. Any S3 endpoint still works.
- **Patroni**: the Zalando Postgres Operator (Patroni inside) was chosen
  over a Bitnami chart (no synchronous failover) and CloudNativePG (not
  Patroni, which the brief names).
- **Canary**: Argo Rollouts was chosen over Flagger (needs a mesh or a
  specific ingress) and a hand-written script (no automatic rollback).
- **Provenance**: the SLSA GitHub generator (level 3, isolated builder)
  was chosen over `actions/attest-build-provenance` (level 2).

## Consequences

- The load, chaos, drill and ZAP jobs turn on by themselves when `serve`
  lands, so the API thread meets the budgets and SLOs from its first day.
- Endpoint paths the load tests use live in one file,
  `deploy/perf/k6/routes.js`, for the API and collab threads to keep true.
- The restore drill drives `vellum backup` and `vellum restore` (#192)
  once the binary has them, and copies the database and bucket directly
  until then. The nightly run also calls `vellum budgets --server` for
  per-route latency budgets (#179) once that command exists.
- Install channels, platforms and desktop signing extend this release in
  [ADR 0060](0060-install-channels-and-platforms.md) (D250-D252).
- GitHub-hosted runners are noisy; if the 10 % gate flaps, move the
  nightly job to a dedicated runner rather than widening the gate.
