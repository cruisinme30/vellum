-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- Background work shared by the replicas of one deployment (ADR 0038,
-- D162-D164). SQLite dialect of migrations/postgres; keep the two in step.
--
-- jobs is the durable queue: any replica enqueues, any replica claims. A
-- claimed job carries a lease (worker, lease_until_ms); when it runs out
-- the job is due again. idem_key deduplicates while a job is ready or
-- running.
--
-- drive_changes lists the pages whose searchable fields changed, written
-- by triggers on drive_nodes, so search follows writes instead of
-- sweeping every page. On SQLite seq orders the changes (one writer at a
-- time, so seq commits in order) and txid stays 0. drive_change_meta holds
-- the newest pruned position: a reader behind it starts over. The
-- triggers number rows past both the newest row and that position, so a
-- number is never reused after a prune.
--
-- cluster_cursors keeps a named reader's position, such as the search
-- feed's, so the next replica to hold its lease carries on from there.

CREATE TABLE jobs (
    id INTEGER NOT NULL PRIMARY KEY,
    queue TEXT NOT NULL CHECK (queue <> ''),
    kind TEXT NOT NULL CHECK (kind <> ''),
    payload TEXT NOT NULL,
    idem_key TEXT NULL,
    state TEXT NOT NULL CHECK (state IN ('ready', 'running', 'done', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    max_attempts INTEGER NOT NULL CHECK (max_attempts > 0),
    run_at_ms INTEGER NOT NULL,
    worker TEXT NULL,
    lease_until_ms INTEGER NULL,
    last_error TEXT NULL,
    traceparent TEXT NULL,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);

CREATE INDEX jobs_due ON jobs (queue, state, run_at_ms, id);
CREATE INDEX jobs_finished ON jobs (state, updated_ms);
CREATE UNIQUE INDEX jobs_open_key ON jobs (queue, idem_key)
WHERE idem_key IS NOT NULL AND state IN ('ready', 'running');

CREATE TABLE drive_changes (
    seq INTEGER NOT NULL PRIMARY KEY,
    txid INTEGER NOT NULL DEFAULT 0,
    node_id BLOB NOT NULL CHECK (length(node_id) = 16),
    at_ms INTEGER NOT NULL
);

CREATE INDEX drive_changes_order ON drive_changes (txid, seq);
CREATE INDEX drive_changes_age ON drive_changes (at_ms);

CREATE TABLE drive_change_meta (
    id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
    horizon_txid INTEGER NOT NULL DEFAULT 0,
    horizon_seq INTEGER NOT NULL DEFAULT 0
);

INSERT INTO drive_change_meta (id) VALUES (1);

-- The SQL linter cannot parse SQLite triggers.
-- noqa: disable=all
CREATE TRIGGER drive_changes_insert AFTER INSERT ON drive_nodes
WHEN new.kind = 'page'
BEGIN
    INSERT INTO drive_changes (seq, node_id, at_ms)
    SELECT
        max(coalesce((SELECT max(seq) FROM drive_changes), 0), horizon_seq) + 1,
        new.node_id,
        cast((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
    FROM drive_change_meta WHERE id = 1;
END;

CREATE TRIGGER drive_changes_update AFTER UPDATE ON drive_nodes
WHEN (old.kind = 'page' OR new.kind = 'page') AND (
    old.kind IS NOT new.kind
    OR old.space_id IS NOT new.space_id
    OR old.tree_parent_id IS NOT new.tree_parent_id
    OR old.path IS NOT new.path
    OR old.title IS NOT new.title
    OR old.byte_size IS NOT new.byte_size
    OR old.modified_at IS NOT new.modified_at
    OR old.trash IS NOT new.trash
)
BEGIN
    INSERT INTO drive_changes (seq, node_id, at_ms)
    SELECT
        max(coalesce((SELECT max(seq) FROM drive_changes), 0), horizon_seq) + 1,
        new.node_id,
        cast((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
    FROM drive_change_meta WHERE id = 1;
END;

CREATE TRIGGER drive_changes_delete AFTER DELETE ON drive_nodes
WHEN old.kind = 'page'
BEGIN
    INSERT INTO drive_changes (seq, node_id, at_ms)
    SELECT
        max(coalesce((SELECT max(seq) FROM drive_changes), 0), horizon_seq) + 1,
        old.node_id,
        cast((julianday('now') - 2440587.5) * 86400000 AS INTEGER)
    FROM drive_change_meta WHERE id = 1;
END;
-- noqa: enable=all

CREATE TABLE cluster_cursors (
    name TEXT NOT NULL PRIMARY KEY CHECK (name <> ''),
    major INTEGER NOT NULL,
    minor INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);
