---
id: 01M63CFE5YV1EYVK287J5GHZ07
title: Introduction
vellum: 1
---

# Admin guide

This guide is for the people who install, secure and run Vellum. It
covers choosing a deployment, configuring storage and sign-in, keys and
encryption, backups, observability, upgrades and capacity.

Vellum is one binary. The same binary runs a single-user workspace on a
laptop and an organisation-wide deployment with replicas, Postgres and
object storage; what changes is configuration, not software.

::: cards

- 💻 **Single** One process, SQLite and a folder. A laptop or a small server.
- 🐳 **Team** Docker Compose with Postgres, object storage and Kroki.
- ☸️ **Org** Kubernetes with Helm: two of everything, a Postgres replica and sharding by space.

:::

## One deployment, one organisation

A Vellum deployment serves exactly one organisation. There is no
multi-tenant mode, so one organisation's data never shares a database
with another's. Inside the deployment, data is partitioned by space,
which is how the Org profile scales out.

## Before you start

::: facts

- Operating system: Linux (x86-64 or ARM64), macOS or Windows for the single binary; Linux for containers
- Memory: under 80 MB idle for the single binary; see [capacity](operate/capacity.md) for larger profiles
- Network: one HTTPS port; Vellum can get its own certificate with ACME
- Storage: a folder, S3-compatible storage, Google Drive, Dropbox, OneDrive or Git

:::

The [user guide](../user.book/README.md) covers using Vellum, and the
[developer guide](../developer.book/README.md) covers its internals.
