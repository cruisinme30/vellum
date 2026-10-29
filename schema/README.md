<!--
SPDX-FileCopyrightText: 2026 The Vellum contributors
SPDX-License-Identifier: AGPL-3.0-only
-->

# Block model schema

The single source for the block model. Rust types and the editor's
ProseMirror schema are generated from it, so the two can never drift.

| File | Role |
| --- | --- |
| `blocks.json` | Every node and mark: name, content expression, attributes, block-ID flag, Markdown mapping and slash-menu entries |
| `blocks.schema.json` | JSON Schema for `blocks.json`, for editor completion |

Run `cargo xtask schema` after editing `blocks.json`. It validates the
cross-references and writes:

- `web/src/editor/schema/blocks.gen.ts`, which the Tiptap editor builds its
  extensions and slash menu from;
- `crates/vellum-contracts/src/blocks.rs`, the `NodeType`, `MarkType` and
  attribute enums the server uses.

CI runs `cargo xtask schema --check` and fails when either file is stale.
Bump `version` when a change needs saved documents to be migrated. See
[ADR 0010](../docs/adr/0010-web-app-and-editor.md).
