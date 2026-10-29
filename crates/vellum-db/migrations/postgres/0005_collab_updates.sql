-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- The live-editing change log (vellum-collab). Unlike the index tables this
-- is owned by the database: it holds edits made since the last Markdown
-- version, so a reindex must never clear it. No foreign keys for the same
-- reason. Each payload is a Yjs update, sealed at rest by the adapter.

CREATE TABLE collab_updates (
    space_id UUID NOT NULL,
    doc_id UUID NOT NULL,
    seq BIGINT NOT NULL CHECK (seq > 0),
    payload BYTEA NOT NULL,
    created_at BIGINT NOT NULL,
    PRIMARY KEY (space_id, doc_id, seq)
);
