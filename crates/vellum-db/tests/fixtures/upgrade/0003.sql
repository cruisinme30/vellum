-- SPDX-FileCopyrightText: 2026 The Vellum contributors
-- SPDX-License-Identifier: AGPL-3.0-only

-- Northstar rows written against the schema as it stood after migration
-- 0003, for the upgrade test (tests/upgrade.rs). Every later migration
-- runs over these rows. ID(n) becomes an engine-specific ID literal: a
-- 16-byte blob on SQLite, a UUID on Postgres. Booleans keep their
-- defaults so one file serves both engines.

INSERT INTO index_meta (key, value) VALUES ('upgrade-fixture', '0003');

INSERT INTO spaces (space_id, kind, slug, root_path, name, indexed_at)
VALUES (
    ID(1),
    'shared_drive',
    'engineering',
    'drives/shared/engineering',
    'Engineering',
    1761696000000
);

INSERT INTO folders (
    space_id, folder_id, parent_id, path, name, icon, indexed_at
)
VALUES (
    ID(1),
    ID(2),
    NULL,
    'drives/shared/engineering/specs',
    'Specs',
    NULL,
    1761696000000
);

INSERT INTO documents (
    space_id, doc_id, folder_id, kind, path, title, status, content_hash,
    byte_size, modified_at, trashed_at, indexed_at
) VALUES
(
    ID(1), ID(3), ID(2), 'page', 'drives/shared/engineering/specs/roadmap.md',
    'Roadmap',
    'verified',
    'sha256:roadmap',
    812,
    1761696000000,
    NULL,
    1761696000000
),
(
    ID(1), ID(4), NULL, 'page', 'drives/shared/engineering/handbook.md',
    'Handbook',
    'draft',
    'sha256:handbook',
    2048,
    1761696000000,
    NULL,
    1761696000000
);

INSERT INTO page_ancestors (space_id, ancestor_id, descendant_id, depth)
VALUES (ID(1), ID(4), ID(4), 0);

INSERT INTO grants (
    space_id, object_type, object_id, relation, subject_type, subject_id,
    expires_at, created_at, created_by
) VALUES (
    ID(1), 'space', ID(1), 'editor', 'user', ID(9), NULL, 1761696000000, ID(9)
);

INSERT INTO effective_access (
    space_id, subject_id, object_id, relation, authz_epoch, valid_until
)
VALUES (ID(1), ID(9), ID(3), 'editor', 0, NULL);

INSERT INTO user_doc_state (
    space_id, user_id, doc_id, starred_at, last_viewed_at, view_count
)
VALUES (ID(1), ID(9), ID(3), 1761696000000, 1761696000000, 3);

INSERT INTO drive_spaces (
    space_id, kind, slug, name, owner_id, root_path, settings
)
VALUES (
    ID(1),
    'shared_drive',
    'engineering',
    'Engineering',
    NULL,
    'drives/shared/engineering',
    '{}'
);

INSERT INTO drive_nodes (
    space_id, node_id, kind, parent_id, tree_parent_id, path, title,
    byte_size, modified_at, created_by, target_id, trash
) VALUES
(
    ID(1),
    ID(2),
    'folder',
    NULL,
    NULL,
    'drives/shared/engineering/specs',
    'Specs',
    0, 1761696000000, ID(9), NULL, NULL
),
(
    ID(1),
    ID(3),
    'page',
    ID(2),
    NULL,
    'drives/shared/engineering/specs/roadmap.md',
    'Roadmap',
    812, 1761696000000, ID(9), NULL, NULL
),
(
    ID(1),
    ID(4),
    'page',
    NULL,
    NULL,
    'drives/shared/engineering/handbook.md',
    'Handbook',
    2048, 1761696000000, ID(9), NULL, NULL
);

INSERT INTO drive_activity (space_id, event_id, node_id, at, entry)
VALUES (ID(1), ID(5), ID(3), 1761696000000, '{"kind":"edited"}');

INSERT INTO drive_activity_scope (space_id, object_id, event_id)
VALUES (ID(1), ID(3), ID(5));

INSERT INTO drive_links (space_id, source_id, target_id)
VALUES (ID(1), ID(4), ID(3));

INSERT INTO drive_versions (space_id, version_id, doc_id, byte_size, author)
VALUES (ID(1), ID(6), ID(3), 790, 'Northstar Admin');
