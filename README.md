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

> Status: before 1.0. No release is tagged yet, so there are no published
> binaries, packages or images: build Vellum from source as shown below.

## Repository layout

| Path | What lives there |
| --- | --- |
| `crates/` | The Rust workspace. `vellum-cli` builds the single `vellum` binary. |
| `web/` | The React and TypeScript web app. |
| `schema/` | The block model, the single source for Rust and the editor. |
| `corpus/` | Golden files that pin the Markdown dialect and every export. |
| `deploy/` | Compose files, the Helm chart and example configuration. |
| `docs/` | Help books in Vellum Markdown and architecture decision records. |
| `site/` | The project website: landing page, docs home, styles, fonts, images and video; `scripts/ci/build-site.sh` builds it with the books and `scripts/ci/check-site.sh` checks it. |
| `xtask/` | Repository automation, such as the crate-boundary check. |

## Build and test

These steps are for macOS on Apple Silicon. On Linux, install the same
tools with your package manager.

### Prerequisites

Install these once:

```sh
xcode-select --install               # cc, make and perl for SQLCipher
curl https://sh.rustup.rs -sSf | sh  # rust-toolchain.toml pins Rust 1.97.0
brew install node@22                 # Node 22 or newer
corepack enable                      # pnpm 10.28, from web/package.json
cargo install cargo-nextest          # the test runner
```

Docker Desktop is needed only for the Docker image.

### Build the web app, then the server

Order matters: the `vellum` binary embeds `web/dist` when it compiles. If
you build the Rust code first, the home page only says "The web app is not
built".

```sh
pnpm --dir web install
pnpm --dir web build                          # writes web/dist
CARGO_BUILD_JOBS=3 cargo build -p vellum-cli  # writes target/debug/vellum
```

`cargo build --release -p vellum-cli` gives a faster binary in
`target/release/vellum`, but takes much longer to compile.

If the build stops because the `sqlx-macros` library can't be loaded (macOS
says the file is corrupt or mis-aligned), run `cargo clean -p sqlx-macros`
and build again. Check that `which strip` prints `/usr/bin/strip`: a
different `strip` earlier on your `PATH` damages the library.

### Run the server

```sh
VELLUM_DATA_DIR=~/vellum-local ./target/debug/vellum serve
```

Open <http://127.0.0.1:7700> and create the first account; it becomes the
workspace owner. The server listens on `127.0.0.1:7700`, keeps its SQLite
index and encryption keys in the data folder, and needs no other settings.
Stop it with Ctrl-C.

### Demo data

A new server starts empty: no command loads the Northstar demo workspace
into `vellum serve`. The demo runs in two places, both without a server:

```sh
pnpm --dir web dev                # web app on http://localhost:5173
./target/debug/vellum mcp --demo  # MCP over stdio, in-memory workspace
```

To work on the web app with hot reload against your running server, point
the dev server at it:

```sh
VELLUM_SERVER=http://127.0.0.1:7700 pnpm --dir web dev
```

### Desktop app

The desktop app loads `web/dist` and starts `target/debug/vellum` itself,
with its own data folder, so build both first:

```sh
pnpm --dir desktop install
pnpm --dir desktop start
```

`pnpm --dir desktop dist` packages an unsigned `.dmg`. It needs the binary
copied to `target/desktop-bin/arm64/vellum` and `VELLUM_BINARY=vellum` set.

### Docker

There is no published image until the first release, so build it locally.
The image builds the web app inside, natively on arm64:

```sh
docker build -f deploy/docker/Dockerfile -t vellum:local .
docker run -p 7700:7700 -v vellum-data:/data vellum:local serve
```

For the team stack (Vellum, Postgres, MinIO, Kroki and ClamAV), see
[deploy/README.md](deploy/README.md). Its `.env.example` already uses
`vellum:local`.

### Tests

```sh
cargo nextest run --workspace
cargo xtask boundaries
pnpm --dir web test
```

[CONTRIBUTING.md](CONTRIBUTING.md) explains the branches, the checks every
pull request must pass and how to sign off commits.

## Licence

Vellum is free software under the
[GNU Affero General Public License v3.0 only](LICENSE). Every file carries
an SPDX header, checked with [REUSE](https://reuse.software/).
