-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- The drive's own index (ADR 0012, D141). SQLite dialect of
-- migrations/postgres; keep the two in step.
-- drive_spaces, drive_nodes, drive_links and drive_versions are derived from
-- the files and rebuilt by the drive's scan. drive_activity and
-- drive_activity_scope are owned by the database. None of these tables
-- references spaces, so `vellum reindex` (which clears spaces) leaves them be.

CREATE TABLE drive_spaces (
    space_id BLOB NOT NULL PRIMARY KEY CHECK (length(space_id) = 16),
    kind TEXT NOT NULL CHECK (kind IN ('shared_drive', 'user_drive')),
    slug TEXT NOT NULL CHECK (slug <> ''),
    name TEXT NOT NULL,
    owner_id BLOB NULL,
    root_path TEXT NOT NULL CHECK (root_path <> ''),
    settings TEXT NOT NULL
);

-- Every drive node, live or trashed. tree_parent_id is the parent the
-- node's access inherits from: its parent, or for a trashed node the
-- folder it was trashed from.
CREATE TABLE drive_nodes (
    space_id BLOB NOT NULL,
    node_id BLOB NOT NULL CHECK (length(node_id) = 16),
    kind TEXT NOT NULL CHECK (
        kind IN ('folder', 'page', 'book', 'collection', 'shortcut')
    ),
    parent_id BLOB NULL,
    tree_parent_id BLOB NULL,
    path TEXT NOT NULL CHECK (path <> ''),
    title TEXT NOT NULL,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    modified_at INTEGER NOT NULL,
    created_by BLOB NULL,
    target_id BLOB NULL,
    trash TEXT NULL,
    PRIMARY KEY (space_id, node_id),
    UNIQUE (node_id)
);
CREATE INDEX drive_nodes_parent ON drive_nodes (space_id, parent_id);
CREATE INDEX drive_nodes_path ON drive_nodes (space_id, path);
CREATE INDEX drive_nodes_title ON drive_nodes (space_id, title);
CREATE INDEX drive_nodes_tree ON drive_nodes (tree_parent_id);

-- Activity entries; event IDs are time-ordered.
CREATE TABLE drive_activity (
    space_id BLOB NOT NULL,
    event_id BLOB NOT NULL CHECK (length(event_id) = 16),
    node_id BLOB NOT NULL,
    at INTEGER NOT NULL,
    entry TEXT NOT NULL,
    PRIMARY KEY (space_id, event_id)
);

-- Which feeds an entry shows in: its node and each of its folders.
CREATE TABLE drive_activity_scope (
    space_id BLOB NOT NULL,
    object_id BLOB NOT NULL,
    event_id BLOB NOT NULL,
    PRIMARY KEY (space_id, object_id, event_id)
);
CREATE INDEX drive_activity_scope_object ON drive_activity_scope (
    object_id, event_id
);

CREATE TABLE drive_links (
    space_id BLOB NOT NULL,
    source_id BLOB NOT NULL,
    target_id BLOB NOT NULL,
    PRIMARY KEY (space_id, source_id, target_id)
);
CREATE INDEX drive_links_source ON drive_links (source_id);
CREATE INDEX drive_links_target ON drive_links (target_id);

CREATE TABLE drive_versions (
    space_id BLOB NOT NULL,
    version_id BLOB NOT NULL CHECK (length(version_id) = 16),
    doc_id BLOB NOT NULL,
    byte_size INTEGER NOT NULL CHECK (byte_size >= 0),
    author TEXT NOT NULL,
    PRIMARY KEY (space_id, version_id),
    UNIQUE (version_id)
);
CREATE INDEX drive_versions_doc ON drive_versions (doc_id, version_id);
