-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- Share links (#95, R-SE-3, ADR 0054 D276). Keep the SQLite and Postgres
-- versions in step.
--
-- A link's token is never stored: token_hash is its SHA-256 in hex, looked
-- up when someone opens /s/<token> on the public origin. The link's access
-- is a viewer grant to share_link:<link_id> in the authorization tables,
-- with the same expiry, so the engine enforces it; this row adds the
-- password (an Argon2id PHC string) and who made the link and when.
-- revoked_at is set once and never cleared.

CREATE TABLE share_links (
    link_id UUID NOT NULL PRIMARY KEY,
    space_id UUID NOT NULL,
    node_id UUID NOT NULL,
    token_hash TEXT NOT NULL UNIQUE CHECK (length(token_hash) = 64),
    password_hash TEXT NULL,
    expires_at BIGINT NULL,
    created_by UUID NOT NULL,
    created_at BIGINT NOT NULL,
    revoked_at BIGINT NULL
);

CREATE INDEX share_links_by_node ON share_links (node_id, created_at);
