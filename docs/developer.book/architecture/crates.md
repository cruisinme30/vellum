---
id: 01M63CFE64BJ2G7XH5ZGSJ7T8R
title: Crate map and layers
vellum: 1
---

# Crate map and layers

Every crate declares a layer in `Cargo.toml`, and may depend only on
crates in strictly lower layers. `cargo xtask boundaries` enforces the
rule in CI, so a service can never reach into another service's
storage.

```toml
[package.metadata.vellum]
layer = "foundation"
```

```mermaid
flowchart LR
  kernel --> contracts --> foundation --> engine --> service --> edge --> app
```

| Layer | Crates | Holds |
| --- | --- | --- |
| kernel | `vellum-kernel` | Typed IDs, errors, events, configuration, telemetry |
| contracts | `vellum-contracts` | Public API types shared by the server and clients |
| foundation | `vellum-format`, `vellum-store`, `vellum-db`, `vellum-help` | The dialect, storage adapters, the index, the built-in help books |
| engine | `vellum-render`, `vellum-authz`, `vellum-collab`, `vellum-export` | Rendering, authorization, real time, export |
| service | `vellum-pages`, `vellum-collections`, `vellum-jobs` | The services |
| edge | `vellum-api`, `vellum-mcp` | HTTP, GraphQL and MCP |
| app | `vellum-cli` | The `vellum` binary |

## Inside a service crate {#service-shape}

```text
crates/vellum-pages/src/
  domain/       # types and rules, no I/O
  ports/        # traits the domain needs: repositories, clocks, queues
  app/          # use cases that orchestrate the domain through ports
  adapters/
    postgres/
    sqlite/
    http/
    events/
```

The domain never imports an adapter. Tests run the app layer against
in-memory adapters, and the same conformance suite runs against every
real adapter.

## Other folders {#folders}

::: facts

- `web/`: the React and Tiptap web app
- `schema/`: the single source of the block model, generating Rust and ProseMirror types
- `corpus/`: golden files for the dialect, rendering and export
- `deploy/`: Compose files, the Helm chart and example environments
- `docs/`: these help books and the architecture decision records
- `xtask/`: repository automation run by CI

:::
