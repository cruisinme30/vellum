-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- The change feed every replica follows (ADR 0038). A replica that writes
-- a file or a space appends a row; the others read the rows past their
-- cursor and drop what they cached about each subject. Keep the SQLite and
-- Postgres files in step.
--
-- cluster_feed is a single row. Every append updates it first, which
-- serialises writers, so positions commit in order and a reader never
-- skips one that commits late (the same rule as sync_meta). Positions at
-- or below horizon were pruned; a reader behind it reloads everything.

CREATE TABLE cluster_feed (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    head INTEGER NOT NULL DEFAULT 0 CHECK (head >= 0),
    horizon INTEGER NOT NULL DEFAULT 0 CHECK (horizon >= 0)
);

INSERT INTO cluster_feed (id) VALUES (1);

CREATE TABLE cluster_changes (
    seq INTEGER NOT NULL PRIMARY KEY CHECK (seq > 0),
    topic TEXT NOT NULL CHECK (topic <> ''),
    subject TEXT NOT NULL,
    origin TEXT NOT NULL
);
