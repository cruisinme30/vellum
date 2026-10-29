-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- Background work shared by the replicas of one deployment (ADR 0038,
-- D162-D164). Keep in step with migrations/sqlite.
--
-- jobs is the durable queue: any replica enqueues, any replica claims
-- with FOR UPDATE SKIP LOCKED. A claimed job carries a lease (worker,
-- lease_until_ms); when it runs out the job is due again. idem_key
-- deduplicates while a job is ready or running.
--
-- drive_changes lists the pages whose searchable fields changed, written
-- by triggers on drive_nodes, so search follows writes instead of
-- sweeping every page. Here seq is allocated before commit, so it can
-- commit out of order; each row also records its transaction (txid), and
-- readers only consume transactions older than every running one
-- (pg_snapshot_xmin), in (txid, seq) order. No lock is shared between
-- writers. drive_change_meta holds the newest pruned position: a reader
-- behind it starts over.
--
-- cluster_cursors keeps a named reader's position, such as the search
-- feed's, so the next replica to hold its lease carries on from there.

CREATE TABLE jobs (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    queue TEXT NOT NULL CHECK (queue <> ''),
    kind TEXT NOT NULL CHECK (kind <> ''),
    payload TEXT NOT NULL,
    idem_key TEXT NULL,
    state TEXT NOT NULL CHECK (state IN ('ready', 'running', 'done', 'failed')),
    attempts BIGINT NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    max_attempts BIGINT NOT NULL CHECK (max_attempts > 0),
    run_at_ms BIGINT NOT NULL,
    worker TEXT NULL,
    lease_until_ms BIGINT NULL,
    last_error TEXT NULL,
    traceparent TEXT NULL,
    created_ms BIGINT NOT NULL,
    updated_ms BIGINT NOT NULL
);

CREATE INDEX jobs_due ON jobs (queue, state, run_at_ms, id);
CREATE INDEX jobs_finished ON jobs (state, updated_ms);
CREATE UNIQUE INDEX jobs_open_key ON jobs (queue, idem_key)
WHERE idem_key IS NOT NULL AND state IN ('ready', 'running');

CREATE TABLE drive_changes (
    seq BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    txid BIGINT NOT NULL DEFAULT (pg_current_xact_id()::TEXT::BIGINT),
    node_id UUID NOT NULL,
    at_ms BIGINT NOT NULL DEFAULT (
        (extract(EPOCH FROM clock_timestamp()) * 1000)::BIGINT
    )
);

CREATE INDEX drive_changes_order ON drive_changes (txid, seq);
CREATE INDEX drive_changes_age ON drive_changes (at_ms);

CREATE TABLE drive_change_meta (
    id SMALLINT NOT NULL PRIMARY KEY CHECK (id = 1),
    horizon_txid BIGINT NOT NULL DEFAULT 0,
    horizon_seq BIGINT NOT NULL DEFAULT 0
);

INSERT INTO drive_change_meta (id) VALUES (1);

CREATE FUNCTION drive_changes_log() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        INSERT INTO drive_changes (node_id) VALUES (old.node_id);
    ELSE
        INSERT INTO drive_changes (node_id) VALUES (new.node_id);
    END IF;
    RETURN NULL;
END;
$$;

CREATE TRIGGER drive_changes_insert AFTER INSERT ON drive_nodes
FOR EACH ROW WHEN (new.kind = 'page')
EXECUTE FUNCTION drive_changes_log();

CREATE TRIGGER drive_changes_update AFTER UPDATE ON drive_nodes
FOR EACH ROW WHEN (
    (old.kind = 'page' OR new.kind = 'page')
    AND (
        old.kind, old.space_id, old.tree_parent_id, old.path, old.title,
        old.byte_size, old.modified_at, old.trash
    )
    IS DISTINCT FROM
    (
        new.kind, new.space_id, new.tree_parent_id, new.path, new.title,
        new.byte_size, new.modified_at, new.trash
    )
)
EXECUTE FUNCTION drive_changes_log();

CREATE TRIGGER drive_changes_delete AFTER DELETE ON drive_nodes
FOR EACH ROW WHEN (old.kind = 'page')
EXECUTE FUNCTION drive_changes_log();

CREATE TABLE cluster_cursors (
    name TEXT NOT NULL PRIMARY KEY CHECK (name <> ''),
    major BIGINT NOT NULL,
    minor BIGINT NOT NULL,
    updated_ms BIGINT NOT NULL
);
