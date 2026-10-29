<!--
SPDX-FileCopyrightText: 2026 The Vellum contributors
SPDX-License-Identifier: AGPL-3.0-only
-->

# Vellum

Vellum is an open-source, self-hosted workspace: documents, books,
collections, issue tracking, personal tasks and a drive, with an MCP server
so models can read, search and draft under your own permissions. Every
object is a Vellum Markdown file in a real folder; the database is an index
you can rebuild.

The project website, with the help books, is at
<https://cruisinme30.github.io/vellum/>.

> Status: repository skeleton. The 1.0 release is built foundation first;
> see [the build order](docs/adr/0001-repository-skeleton.md).

## Repository layout

| Path | What lives there |
| --- | --- |
| `crates/` | The Rust workspace. `vellum-cli` builds the single `vellum` binary. |
| `web/` | The React and TypeScript web app. |
| `schema/` | The block model, the single source for Rust and the editor. |
| `corpus/` | Golden files that pin the Markdown dialect and every export. |
| `deploy/` | Compose files, the Helm chart and example configuration. |
| `docs/` | Help books in Vellum Markdown and architecture decision records. |
| `site/` | The project website's landing page; `scripts/ci/build-site.sh` adds the books. |
| `xtask/` | Repository automation, such as the crate-boundary check. |

## Build and test

You need Rust (the version in `rust-toolchain.toml` installs itself),
Node 22 and pnpm 10.

```sh
cargo build
cargo nextest run --workspace
cargo xtask boundaries
pnpm --dir web install
pnpm --dir web test
```

[CONTRIBUTING.md](CONTRIBUTING.md) explains the branches, the checks every
pull request must pass and how to sign off commits.

## Licence

Vellum is free software under the
[GNU Affero General Public License v3.0 only](LICENSE). Every file carries
an SPDX header, checked with [REUSE](https://reuse.software/).
