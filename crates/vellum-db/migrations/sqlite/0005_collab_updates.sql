-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- The live-editing change log (vellum-collab). Unlike the index tables this
-- is owned by the database: it holds edits made since the last Markdown
-- version, so a reindex must never clear it. No foreign keys for the same
-- reason. Each payload is a Yjs update, sealed at rest by the adapter.
-- SQLite dialect of migrations/postgres; keep the two in step.

CREATE TABLE collab_updates (
    space_id BLOB NOT NULL CHECK (length(space_id) = 16),
    doc_id BLOB NOT NULL CHECK (length(doc_id) = 16),
    seq INTEGER NOT NULL CHECK (seq > 0),
    payload BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (space_id, doc_id, seq)
);
