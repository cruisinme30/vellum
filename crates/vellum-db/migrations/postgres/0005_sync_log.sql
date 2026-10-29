-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- The sync service's change log (vellum-sync, ADR 0029). Postgres dialect of
-- migrations/sqlite; keep the two in step. Numbered 0005 so it does not
-- clash with the shard map (0004) being added in parallel.
--
-- sync_meta is a single row. Every append locks it first, which serialises
-- writers, so log positions commit in order and a reader never skips a
-- position that commits late. The epoch starts at the creation time in
-- milliseconds, so a rebuilt index never accepts a cursor from the old one.

CREATE TABLE sync_meta (
    id SMALLINT NOT NULL PRIMARY KEY CHECK (id = 1),
    epoch BIGINT NOT NULL CHECK (epoch > 0),
    head BIGINT NOT NULL DEFAULT 0 CHECK (head >= 0),
    horizon BIGINT NOT NULL DEFAULT 0 CHECK (horizon >= 0)
);

INSERT INTO sync_meta (id, epoch)
VALUES (1, (EXTRACT(EPOCH FROM CLOCK_TIMESTAMP()) * 1000)::BIGINT);

-- The latest version of each record at the position of its last write;
-- value is JSON text (not jsonb, so key order survives), NULL for a tombstone.
CREATE TABLE sync_log (
    seq BIGINT NOT NULL PRIMARY KEY CHECK (seq > 0),
    space_id UUID NOT NULL,
    object_id UUID NULL,
    collection TEXT NOT NULL CHECK (collection <> ''),
    record_key TEXT NOT NULL,
    value TEXT NULL,
    UNIQUE (space_id, collection, record_key)
);

-- One row per device: the account it is bound to and the last mutation
-- processed for it.
CREATE TABLE sync_clients (
    client_id TEXT NOT NULL PRIMARY KEY CHECK (client_id <> ''),
    user_id UUID NOT NULL,
    last_mutation_id BIGINT NOT NULL CHECK (last_mutation_id >= 0),
    updated_at BIGINT NOT NULL
);
