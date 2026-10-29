-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- Sign-in records shared by every replica (vellum-identity, ADR 0037, D160).
-- SQLite dialect of migrations/postgres; keep the two in step.
--
-- Unlike the rest of the index these rows are not rebuildable from files:
-- they are the accounts, sessions, tokens and lockout counters themselves.
--
-- identity_meta is a single row. Every write bumps version first, which
-- serialises writers. Each record row carries the version that last wrote
-- it, so a reader refreshes its cache with `version > <what it has>`.
-- Deletes leave a tombstone (record NULL); old tombstones are purged and
-- horizon moves past them, and a cache older than horizon reloads everything.

CREATE TABLE identity_meta (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    horizon INTEGER NOT NULL DEFAULT 0 CHECK (horizon >= 0)
);

INSERT INTO identity_meta (id) VALUES (1);

-- kind: user, session, api_token, grant, access_token, lockout,
-- oauth_consent or oauth_code.
-- record is the JSON of the record, NULL for a tombstone. Secrets appear
-- only as hashes, passwords only as Argon2id hashes, lockout keys only as
-- SHA-256 of the address.
CREATE TABLE identity_records (
    kind TEXT NOT NULL CHECK (kind <> ''),
    id TEXT NOT NULL CHECK (id <> ''),
    version INTEGER NOT NULL CHECK (version > 0),
    record TEXT NULL,
    PRIMARY KEY (kind, id)
);

CREATE INDEX identity_records_version ON identity_records (version);
