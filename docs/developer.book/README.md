---
id: 01M63CFE63GFD2V5SWXEVC6X36
title: Introduction
vellum: 1
---

# Developer guide

This guide is for people who build on Vellum or build Vellum itself: the
architecture, the crate layout, the Markdown dialect's grammar and block
model, the REST, GraphQL, webhook and MCP interfaces, and how to extend
and contribute.

::: facts

- Server: Rust (axum, tokio, sqlx), one binary
- Index: SQLite locally, Postgres in the team and org profiles
- Editor: TypeScript, React 19, Vite and Tiptap
- Real time: Yjs on the client, yrs on the server
- Licence: AGPL-3.0-only, with a Developer Certificate of Origin sign-off on every commit

:::

## Principles

::: cards

- 📄 **Files are the truth** Every object is a Vellum Markdown file; the database is a rebuildable index.
- 🧱 **Ports and adapters** Services own their data behind traits; adapters plug in storage, databases and transports.
- 🚫 **Deny by default** Every read and write passes one authorization check.
- 🔭 **Observable from the first commit** Every I/O path has a span and metrics.

:::

Start with [Architecture](architecture/README.md), then read
[Contributing](contributing/README.md) before your first pull request.
