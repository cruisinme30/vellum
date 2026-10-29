<!--
SPDX-FileCopyrightText: 2026 The Vellum contributors
SPDX-License-Identifier: AGPL-3.0-only
-->

# Deployment

The deployment ladder, from smallest to largest: the single binary
(SQLite and a folder), `docker run`, `docker compose` (Postgres, MinIO,
Kroki), and the Helm chart (two of everything, a Postgres replica, shards by
space). Each rung lands here with its workstream.
