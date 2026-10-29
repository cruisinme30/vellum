-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- Which shard holds each space. Lives in the main database; a space with no
-- row is on the home shard (the first name in VELLUM_SHARDS). Unused when
-- the deployment has one database.
CREATE TABLE space_shards (
    space_id UUID NOT NULL PRIMARY KEY,
    shard TEXT NOT NULL CHECK (shard <> ''),
    assigned_at BIGINT NOT NULL
);
CREATE INDEX space_shards_shard ON space_shards (shard);
