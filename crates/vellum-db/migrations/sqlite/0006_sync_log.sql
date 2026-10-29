-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- The sync service's change log (vellum-sync, ADR 0029). SQLite dialect of
-- migrations/postgres; keep the two in step.
--
-- sync_meta is a single row. Every append updates it first, which
-- serialises writers, so log positions commit in order and a reader never
-- skips a position that commits late. The epoch starts at the creation time
-- in milliseconds, so a rebuilt index never accepts a cursor from the old one.

CREATE TABLE sync_meta (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    epoch INTEGER NOT NULL CHECK (epoch > 0),
    head INTEGER NOT NULL DEFAULT 0 CHECK (head >= 0),
    horizon INTEGER NOT NULL DEFAULT 0 CHECK (horizon >= 0)
);

INSERT INTO sync_meta (id, epoch)
VALUES (1, CAST((JULIANDAY('now') - 2440587.5) * 86400000 AS INTEGER));

-- The latest version of each record at the position of its last write;
-- value is JSON text, NULL for a tombstone.
CREATE TABLE sync_log (
    seq INTEGER NOT NULL PRIMARY KEY CHECK (seq > 0),
    space_id BLOB NOT NULL CHECK (LENGTH(space_id) = 16),
    object_id BLOB NULL CHECK (object_id IS NULL OR LENGTH(object_id) = 16),
    collection TEXT NOT NULL CHECK (collection <> ''),
    record_key TEXT NOT NULL,
    value TEXT NULL,
    UNIQUE (space_id, collection, record_key)
);

-- One row per device: the account it is bound to and the last mutation
-- processed for it.
CREATE TABLE sync_clients (
    client_id TEXT NOT NULL PRIMARY KEY CHECK (client_id <> ''),
    user_id BLOB NOT NULL CHECK (LENGTH(user_id) = 16),
    last_mutation_id INTEGER NOT NULL CHECK (last_mutation_id >= 0),
    updated_at INTEGER NOT NULL
);
