---
id: 01M64V9FCMPDTZZXRCQX0QZ3YM
title: "ADR 0064: Security gates for 1.0"
status: draft
owner: "@cruisinme30"
tags: [adr, security, crypto, supply-chain]
vellum: 1
---

::: facts

- Date: 2026-10-29
- Status: accepted
- Decisions: D259, D260, D261
- Issue: [#514](https://github.com/cruisinme30/vellum/issues/514), and
  [#195](https://github.com/cruisinme30/vellum/issues/195) for the pen test
- Pull requests: the integration batch that carries `feat/security-gates`

:::

## Context

The brief (§11) makes envelope encryption at rest the default, with the
key-encryption key (KEK) in a key service, the OS keychain or a
passphrase. [ADR 0002](0002-envelope-encryption.md) built the envelope
format and `vellum-sealed-store`, and `vellum-kms` built the key service
adapters. Three gaps were left for 1.0:

- `vellum serve` never wrapped its store in `Sealed`, so workspace files
  were stored in plaintext. The Compose file and the Helm chart passed a
  KEK passphrase that nothing read.
- `AwsKms` signed requests with fixed access keys only. Clusters on AWS
  should use a role, so no long-lived key sits in a secret.
- OpenSSF Scorecard rated the repository 6.2. Its Vulnerabilities check
  counted advisories we had already triaged in `deny.toml`, and the
  hygiene job installed Python linters without hashes.

## Decisions

### `D259`: AWS KMS takes credentials from the SDK chain

- `AwsCredentialProvider::from_env` tries, in the AWS SDK order: access
  keys in the environment, web identity (EKS IRSA), container credentials
  (ECS task roles, EKS Pod Identity), then IMDSv2.
- The web identity token and the Pod Identity token are read from their
  files on every refresh, because Kubernetes rotates them. The STS call
  sends the token in a form body, never in a URL.
- A full container-credentials URI must be HTTPS, or plain HTTP to
  loopback or the ECS and EKS agents, so the token never crosses a network
  in clear. IMDS is version 2 only.
- Temporary credentials are cached and refreshed five minutes before they
  expire. They are wiped from memory after use and never in `Debug`
  output or errors.
- `RemoteKeys::from_kek_id` picks AWS KMS, Cloud KMS (metadata server
  identity) or Vault transit from one KEK id.

### `D260`: `vellum serve` seals every file by default

- The KEK is the first of: `VELLUM_KMS_KEY` (a key service KEK id), the
  passphrase in `VELLUM_KEK_PASSPHRASE` or `VELLUM_KEK_PASSPHRASE_FILE`,
  or a random key in `.vellum/keys/files.kek`. A shared bucket refuses
  to start without a key service or passphrase, because replicas cannot
  share a key file.
- The passphrase KEK's salt and costs are kept in the store at
  `.vellum/kek`, written once with `IfAbsent`, so every replica and every
  restart wraps under one KEK. Other passphrase KEK ids are derived on
  first use, capped at four times the default Argon2id memory.
- The key file's id carries a fingerprint, recorded at `.vellum/kek-file`.
  A missing or different key file stops startup instead of making a new
  key that cannot open the existing files.
- `VELLUM_FILES_PLAINTEXT=1` stores files as they are, for people who edit
  the data folder with other tools. Only then does the file watcher run.
- Reads accept sealed and plaintext objects, so existing workspaces keep
  working. A background pass at startup seals the old plaintext files in
  the drives, version history and trash, and re-wraps envelopes under an
  older KEK. Each rewrite is conditional, so a concurrent edit wins. The
  KEK id is recorded at `.vellum/sealed` once a pass completes.
- E2E recipient envelopes are stored as the clients sealed them, and
  `vellum reindex` opens sealed files with the same keys as the server.

### `D261`: Scorecard findings are fixed or recorded with a reason

- The hygiene job installs its linters from
  `.github/requirements/hygiene.txt`, every package pinned by hash, and
  Dependabot updates the file.
- `osv-scanner.toml` lists each triaged advisory with its reason. The
  RUSTSEC entries mirror `deny.toml` and `.cargo/audit.toml`.
- `SECURITY.md` links the private reporting form, the advisories page and
  the ASVS self-assessment.
- The SLSA generator stays referenced by tag, because its reusable
  workflows must be. Code review, branch protection and the OpenSSF Best
  Practices badge need the maintainer; the thread lists what to do.

## Alternatives

- **Keep the data folder in plaintext by default** and seal only buckets:
  files stay editable in any editor, but the brief makes encryption the
  default and plaintext the explicit choice.
- **Per-space plaintext through the pages service's `ModePolicy`**: the
  brief's end state, but no space setting exists yet. One store-wide
  switch ships now; a per-space policy can replace `PrefixPolicy` later.
- **The official AWS SDK for credentials**: dozens of crates for what four
  small HTTP calls do, next to a SigV4 signer we already have.
- **A fresh random salt per start for passphrase KEKs**: every restart
  would add a KEK id, and each one costs an Argon2id derivation on read.

## Consequences

- New installs and upgraded ones seal files from the first write. Back
  up `.vellum/keys/files.kek`, or use a key service: backups never carry
  the key folder.
- With encryption on, edits made outside Vellum are not picked up. Admin
  docs say so and name the plaintext setting.
- The Helm chart takes `encryption.kms`; with it set, no passphrase secret
  is needed and the pod's role reaches the key.
- Vault still uses a static token (`VAULT_TOKEN`); Kubernetes auth for
  Vault is a follow-up.
- The ASVS page marks these controls met. The maintainer deferred the
  external pen test past 1.0 on 2026-10-29; its scope stays ready in
  `docs/security/pen-test-scope.md`, and internal red teaming covers 1.0.
