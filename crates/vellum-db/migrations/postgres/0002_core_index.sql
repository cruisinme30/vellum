-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- Core index tables. Everything here is derived from the files on disk except
-- grants and user_doc_state, which the database owns (grants keep a YAML copy).
-- IDs are vellum-kernel IDs (uuid in Postgres, 16-byte blob in SQLite).
-- Timestamps are UTC epoch milliseconds. space_id is the shard key.

CREATE TABLE spaces (
    space_id UUID NOT NULL PRIMARY KEY,
    kind TEXT NOT NULL CHECK (
        kind IN ('shared_drive', 'user_drive', 'team', 'tasks')
    ),
    slug TEXT NOT NULL CHECK (slug <> ''),
    root_path TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    plaintext BOOLEAN NOT NULL DEFAULT FALSE,
    authz_epoch BIGINT NOT NULL DEFAULT 0,
    indexed_at BIGINT NOT NULL,
    UNIQUE (kind, slug)
);

CREATE TABLE folders (
    space_id UUID NOT NULL REFERENCES spaces (space_id) ON DELETE CASCADE,
    folder_id UUID NOT NULL,
    parent_id UUID NULL,
    path TEXT NOT NULL CHECK (path <> ''),
    name TEXT NOT NULL,
    icon TEXT NULL,
    indexed_at BIGINT NOT NULL,
    PRIMARY KEY (space_id, folder_id),
    UNIQUE (space_id, path),
    UNIQUE (folder_id),
    FOREIGN KEY (space_id, parent_id) REFERENCES folders (
        space_id, folder_id
    ) ON DELETE CASCADE
);

CREATE TABLE documents (
    space_id UUID NOT NULL REFERENCES spaces (space_id) ON DELETE CASCADE,
    doc_id UUID NOT NULL,
    folder_id UUID NULL REFERENCES folders (folder_id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK (
        kind IN (
            'page',
            'book',
            'collection',
            'row',
            'issue',
            'task_list',
            'shortcut'
        )
    ),
    path TEXT NOT NULL CHECK (path <> ''),
    title TEXT NOT NULL,
    status TEXT NULL CHECK (
        status IN ('draft', 'in-review', 'verified', 'outdated')
    ),
    content_hash TEXT NOT NULL,
    byte_size BIGINT NOT NULL CHECK (byte_size >= 0),
    modified_at BIGINT NOT NULL,
    trashed_at BIGINT NULL,
    explicitly_trashed BOOLEAN NOT NULL DEFAULT FALSE,
    indexed_at BIGINT NOT NULL,
    PRIMARY KEY (space_id, doc_id),
    UNIQUE (space_id, path)
);
CREATE UNIQUE INDEX documents_doc_id ON documents (doc_id);
CREATE INDEX documents_folder ON documents (space_id, folder_id);
CREATE INDEX documents_modified ON documents (space_id, modified_at DESC);

-- Closure table over folders and documents (both are nodes with kernel IDs).
-- Every node has a depth-0 row to itself, so one lookup covers
-- "this node or any of its ancestors".
CREATE TABLE page_ancestors (
    space_id UUID NOT NULL REFERENCES spaces (space_id) ON DELETE CASCADE,
    ancestor_id UUID NOT NULL,
    descendant_id UUID NOT NULL,
    depth INTEGER NOT NULL CHECK (depth >= 0),
    PRIMARY KEY (space_id, descendant_id, ancestor_id)
);
CREATE INDEX page_ancestors_ancestor ON page_ancestors (
    space_id, ancestor_id, depth
);

-- Zanzibar-shaped relation tuples: object#relation@subject[#subject_relation].
-- Owned by the authorization service; reindex never deletes them.
CREATE TABLE grants (
    space_id UUID NOT NULL,
    object_type TEXT NOT NULL CHECK (object_type <> ''),
    object_id UUID NOT NULL,
    relation TEXT NOT NULL CHECK (relation <> ''),
    subject_type TEXT NOT NULL CHECK (subject_type <> ''),
    subject_id UUID NOT NULL,
    subject_relation TEXT NOT NULL DEFAULT '',
    expires_at BIGINT NULL,
    created_at BIGINT NOT NULL,
    created_by UUID NULL,
    PRIMARY KEY (
        space_id,
        object_type,
        object_id,
        relation,
        subject_type,
        subject_id,
        subject_relation
    )
);
CREATE INDEX grants_subject ON grants (space_id, subject_type, subject_id);

-- Materialised effective access, stamped with the space's authz epoch
-- it was computed at.
CREATE TABLE effective_access (
    space_id UUID NOT NULL,
    subject_id UUID NOT NULL,
    object_id UUID NOT NULL,
    relation TEXT NOT NULL,
    authz_epoch BIGINT NOT NULL,
    PRIMARY KEY (space_id, subject_id, object_id, relation)
);
CREATE INDEX effective_access_subject ON effective_access (
    subject_id, object_id
);

-- Per-user state the files do not carry: stars and recents. Database-owned.
CREATE TABLE user_doc_state (
    space_id UUID NOT NULL,
    user_id UUID NOT NULL,
    doc_id UUID NOT NULL,
    starred_at BIGINT NULL,
    last_viewed_at BIGINT NULL,
    view_count BIGINT NOT NULL DEFAULT 0 CHECK (view_count >= 0),
    PRIMARY KEY (space_id, user_id, doc_id)
);
CREATE INDEX user_doc_state_recent ON user_doc_state (
    user_id, last_viewed_at DESC
);
CREATE INDEX user_doc_state_starred ON user_doc_state (
    user_id, starred_at
) WHERE starred_at IS NOT NULL;
