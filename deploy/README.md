<!--
SPDX-FileCopyrightText: 2026 The Vellum contributors
SPDX-License-Identifier: AGPL-3.0-only
-->

# Deployment

The deployment ladder, from smallest to largest (ADR 0022). Every rung runs
the same signed binary.

| Rung | What runs | Where |
| --- | --- | --- |
| Single binary | `vellum serve`, SQLite and a folder | Homebrew, winget, `.deb`/`.rpm`, [`install.sh`](../site/install.sh) or a release download ([`packages/`](packages/nfpm.yaml)) |
| `docker run` | the same binary in a 4 MB distroless image | [`docker/`](docker/Dockerfile) |
| Team | Vellum, Postgres, MinIO, Kroki, ClamAV | [`compose/`](compose/compose.yaml) |
| Organisation | two of everything, a Patroni replica per shard, canary releases | [`helm/vellum/`](helm/vellum/values.yaml) |

## Single binary and `docker run`

```sh
docker run -p 7700:7700 -v vellum-data:/data ghcr.io/cruisinme30/vellum:latest
```

Build the image yourself from the repository root:

```sh
docker build -f deploy/docker/Dockerfile -t vellum .
```

## Team: docker compose

```sh
cp deploy/compose/.env.example deploy/compose/.env    # set every change-me
openssl rand -base64 32 > deploy/compose/secrets/kek-passphrase
openssl rand -base64 32 > deploy/compose/secrets/csrf-key
docker compose -f deploy/compose/compose.yaml up -d
```

The passphrase file wraps every encryption key. Back it up apart from the
volumes: without it the data cannot be read. The CSRF key signs form
tokens; every Vellum container must read the same one (an existing install
upgrading to this version creates it once with the command above).
Accounts and sign-in sessions live in Postgres, so they survive restarts
and are shared by every replica; an `identity.json` from a single-binary
install is imported on first start.

## Organisation: Helm

Install the [Zalando Postgres Operator](https://github.com/zalando/postgres-operator)
first, and [Argo Rollouts](https://argoproj.github.io/rollouts/) for canary
releases. Then:

```sh
kubectl create secret generic vellum-kek --from-literal=passphrase="$(openssl rand -base64 32)"
helm install vellum deploy/helm/vellum -f deploy/helm/vellum/values-production.yaml
```

Each entry in `shards` gets its own Postgres cluster; spaces are spread
across them. Add shards, never rename or remove them.

## Verifying a release

Every file in a release has a Sigstore bundle and SLSA provenance, and the
image is signed:

```sh
cosign verify ghcr.io/cruisinme30/vellum:1.0.0 \
  --certificate-identity-regexp '^https://github.com/cruisinme30/vellum/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

The release notes carry the matching commands for the binaries.

## Test harnesses

| Harness | When | Entry point |
| --- | --- | --- |
| Budgets: size, cold start, idle memory, drain | Full suite | `cargo xtask budgets --binary <path>` |
| Reproducible build | Full suite and every release | two builds, byte-compared |
| Load test on two replicas, 10 % p95 gate | nightly | [`perf/k6/suite.js`](perf/k6/suite.js), `cargo xtask perf-gate` |
| Write on one replica, read on the other | nightly | [`perf/k6/replicas.js`](perf/k6/replicas.js) |
| ZAP baseline scan | nightly | [`zap/rules.tsv`](zap/rules.tsv) |
| Chaos under load | weekly | [`chaos/run.sh`](chaos/run.sh) |
| Upgrade and restore drill | every release | [`drills/`](drills/upgrade.sh) |

The load, chaos, ZAP and drill jobs report a notice and pass until the
binary has a `serve` command. The load scripts sign in as they start: on
an empty server they set up the administrator, then make a short-lived API
token (or use `TOKEN`). `REPLICAS=2 deploy/compose/stack.sh up IMAGE
PROJECT PORT` starts a second Vellum on `PORT+1` over the same Postgres and
bucket.
