---
id: 01M64X19XYJXKMNA27577EA67N
title: "ADR 0044: S3 workload identity, reindex from the bucket and degraded modes"
status: verified
owner: "@cruisinme30"
tags: [adr, store, deploy]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D186, D187, D188, D189
- Issue: #476, #477, #191
- Pull requests: to be linked when merged

:::

## Context

ADR 0037 moved the workspace files into a shared S3 bucket so several
replicas can serve one workspace. Three gaps were left for 1.0. The S3
adapter could only sign with static keys, although the Helm chart already
promised workload identity. `vellum reindex`, the recovery path for a
lost index (ADR 0006, D48), only walked a folder on disk, so a workspace
in a bucket could not be reindexed. And a bucket outage failed `/readyz`
on every replica at once, which took the whole service down, index-backed
reads included (#191).

## Decisions

### D186: S3 keys come from the AWS credential chain

- The adapter takes a `CredentialSource`: static keys, STS web identity,
  the container credential endpoint, or the EC2 instance profile.
- `from_vars` picks the first one set up, in the AWS SDK order: static
  keys, then `AWS_ROLE_ARN` with `AWS_WEB_IDENTITY_TOKEN_FILE`, then
  `AWS_CONTAINER_CREDENTIALS_*`, then IMDSv2.
- Temporary keys are cached and refreshed five minutes before they
  expire, one refresh at a time. A failed refresh keeps using keys that
  are still valid.
- Token files are read again on every refresh, because the kubelet
  rotates them.
- STS is reached at the regional endpoint, or at `AWS_ENDPOINT_URL_STS`,
  which also lets MinIO's STS serve any Kubernetes cluster.

### D187: reindex scans a listing of the bucket

- The scanner in `vellum-db` walks a `Tree`: the folder on disk, or a
  `Listing` held in memory. Both go through the same code, so a bucket
  and a folder give the same rows.
- `vellum reindex` lists the bucket once when `VELLUM_BLOB_URL` is set,
  fetches only the Markdown and YAML outside `.vellum/`, sixteen at a
  time, and writes one transaction as before (per shard when sharded).
- A file deleted between the listing and its fetch is left out.

### D188: a shared-store outage is degraded, not unready

- The storage probe reports `degraded` when the shared bucket does not
  answer, and `not_ready` only for a data folder on this machine.
- Books and collections, which load from the files, report `degraded`
  when that load fails.
- Calls that need the files still fail with `unavailable` and a
  `Retry-After`, through the gateway's breaker.
- The weekly chaos test cuts the servers off the bucket for twenty
  seconds and checks every acknowledged write.

### D189: credential endpoints are pinned

- A full container credential URL must be `https`, loopback, or the ECS
  or EKS Pod Identity agent address, as the AWS SDKs require, so a
  planted variable cannot send the token elsewhere.
- The instance profile is only tried when the bucket is on AWS itself:
  with a custom endpoint (MinIO, R2) and no keys, startup fails clearly
  instead of probing `169.254.169.254`.
- Errors name the source and STS's error code, never a token or key.

## Alternatives

- The AWS SDK for Rust. It brings a second HTTP and TLS stack and a large
  dependency tree for four small requests the adapter can make itself.
- Mirroring the bucket to a temporary folder for reindex. It needs disk
  for the whole workspace, attachments included, and a cleanup step.
- Removing storage from `/readyz` altogether. A replica whose local data
  folder is gone must still leave the load balancer.

## Consequences

- On EKS, annotating the service account is all the S3 setup needed, and
  no long-lived key is stored in the cluster.
- A reindex keeps every page and manifest of the workspace in memory
  while it runs; for very large workspaces, run it on a machine sized
  for that.
- During a bucket outage, pages cannot be opened or saved, but search,
  lists, Work and My Work keep answering.
- Redis is not part of Vellum: the parts #191 listed for it (rate limits,
  jobs, the change feed) live in Postgres since ADR 0037 and 0038.
- `vellum-kms` still reads static keys for AWS KMS; moving it to the same
  chain is a follow-up.
