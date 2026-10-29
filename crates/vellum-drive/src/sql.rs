// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! [`SqlIndex`]: the [`DriveIndex`] on the `vellum-db` index (D141).
//!
//! The drive reads only its own tables (`drive_*`, migration 0003), so a
//! `vellum reindex`, which clears the shared tables, never loses drive
//! state. Every write also keeps the shared tables in step, so the rest of
//! Vellum sees the drive without waiting for a reindex:
//!
//! - `spaces`, `folders` (live folders) and `documents` (pages, books,
//!   collections and shortcuts, trashed ones with `trashed_at`);
//! - `page_ancestors`, the closure the authorization service reads, in the
//!   shape a reindex writes: the space is the root, with a row to itself,
//!   above every top-level node. A trashed node keeps the folder it was
//!   trashed from, as in [`crate::authz`], and a node whose parent is gone
//!   becomes top level;
//! - `user_doc_state` for stars and recents.
//!
//! The SQL is written once: `$n` placeholders and `ON CONFLICT` read the
//! same on SQLite and Postgres, and each method body is expanded for both
//! pools by the `on_pool` macro.

use std::collections::HashSet;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use vellum_db::sqlx::types::Uuid;
use vellum_db::{Index, IndexPool, sqlx, traced};
use vellum_kernel::{Actor, EventId, ObjectId, SpaceId, Timestamp, UserId, VersionId};
use vellum_store::BlobPath;

use crate::error::{DriveError, DriveResult};
use crate::files;
use crate::model::{
    Activity, ActivityKind, DriveTotals, Node, NodeKind, Space, SpaceKind, SpaceSettings,
    TrashState, VersionRecord,
};
use crate::ports::DriveIndex;

/// Depth past which an ancestor chain is treated as a cycle and cut.
const MAX_DEPTH: i32 = 4096;

/// `SELECT` of a full node row from `drive_nodes`, filtered by `$filter`.
macro_rules! select_nodes {
    ($filter:literal) => {
        concat!(
            "SELECT space_id, node_id, kind, parent_id, path, title, byte_size, \
             modified_at, created_by, target_id, trash FROM drive_nodes WHERE ",
            $filter
        )
    };
}

/// A [`DriveIndex`] stored in the `vellum-db` index, on SQLite or Postgres.
#[derive(Clone)]
pub struct SqlIndex {
    db: Index,
}

impl fmt::Debug for SqlIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqlIndex")
            .field("backend", &self.db.backend())
            .finish_non_exhaustive()
    }
}

impl SqlIndex {
    /// Wraps an opened (and therefore migrated) index.
    #[must_use]
    pub const fn new(db: Index) -> Self {
        Self { db }
    }

    /// The underlying index.
    #[must_use]
    pub const fn db(&self) -> &Index {
        &self.db
    }
}

/// Expands `$body` once per backend, with `$pool` bound to that backend's
/// pool and `$system` to its `OpenTelemetry` `db.system`.
macro_rules! on_pool {
    ($self:ident, $pool:ident, $system:ident => $body:block) => {
        match $self.db.pool() {
            IndexPool::Sqlite($pool) => {
                let $system = "sqlite";
                $body
            }
            IndexPool::Postgres($pool) => {
                let $system = "postgresql";
                $body
            }
            _ => Err(DriveError::Index("unsupported index backend".to_owned())),
        }
    };
}

/// Runs one statement under a `db.query` span and maps its error.
macro_rules! run {
    ($system:ident, $op:literal, $fut:expr) => {
        traced($system, concat!("drive.", $op), $fut)
            .await
            .map_err(db_error)?
    };
}

/// Inserts or refreshes a space's row in the shared `spaces` table from
/// `drive_spaces`. Evaluates to whether the row now exists.
macro_rules! project_space {
    ($tx:ident, $system:ident, $space:expr, $now:expr) => {{
        let space: Uuid = $space;
        let row: Option<(String, String, String, String)> = run!(
            $system,
            "space.read",
            sqlx::query_as(
                "SELECT kind, slug, name, root_path FROM drive_spaces WHERE space_id = $1"
            )
            .bind(space)
            .fetch_optional(&mut *$tx)
        );
        if let Some((kind, slug, name, root)) = row {
            // A stale row holding the same slug or directory goes first.
            let _ = run!(
                $system,
                "space.clear_stale",
                sqlx::query(
                    "DELETE FROM spaces WHERE space_id <> $1 \
                     AND (root_path = $2 OR (kind = $3 AND slug = $4))",
                )
                .bind(space)
                .bind(&root)
                .bind(&kind)
                .bind(&slug)
                .execute(&mut *$tx)
            );
            let _ = run!(
                $system,
                "space.project",
                sqlx::query(
                    "INSERT INTO spaces (space_id, kind, slug, root_path, name, indexed_at) \
                     VALUES ($1, $2, $3, $4, $5, $6) \
                     ON CONFLICT (space_id) DO UPDATE SET kind = excluded.kind, \
                     slug = excluded.slug, root_path = excluded.root_path, \
                     name = excluded.name, indexed_at = excluded.indexed_at",
                )
                .bind(space)
                .bind(&kind)
                .bind(&slug)
                .bind(&root)
                .bind(&name)
                .bind($now)
                .execute(&mut *$tx)
            );
            let _ = run!(
                $system,
                "space.closure",
                sqlx::query(
                    "INSERT INTO page_ancestors (space_id, ancestor_id, descendant_id, depth) \
                     VALUES ($1, $1, $1, 0) ON CONFLICT DO NOTHING",
                )
                .bind(space)
                .execute(&mut *$tx)
            );
            true
        } else {
            false
        }
    }};
}

/// Rebuilds the `page_ancestors` rows of `$root` and of every node beneath
/// it (by `tree_parent_id`), in `$space`.
macro_rules! rebuild_closure {
    ($tx:ident, $system:ident, $root:expr, $space:expr) => {{
        let root: Uuid = $root;
        let space: Uuid = $space;
        let members: Vec<(Uuid,)> = run!(
            $system,
            "closure.subtree",
            sqlx::query_as(
                "WITH RECURSIVE sub(id) AS (SELECT $1 UNION \
                 SELECT n.node_id FROM drive_nodes n JOIN sub ON n.tree_parent_id = sub.id) \
                 SELECT id FROM sub",
            )
            .bind(root)
            .fetch_all(&mut *$tx)
        );
        for (member,) in members {
            let _ = run!(
                $system,
                "closure.clear",
                sqlx::query("DELETE FROM page_ancestors WHERE descendant_id = $1")
                    .bind(member)
                    .execute(&mut *$tx)
            );
            let chain: Vec<(Uuid, i32)> = run!(
                $system,
                "closure.chain",
                sqlx::query_as(
                    "WITH RECURSIVE chain(id, depth) AS (SELECT $1, 0 UNION ALL \
                     SELECT p.node_id, c.depth + 1 FROM chain c \
                     JOIN drive_nodes n ON n.node_id = c.id \
                     JOIN drive_nodes p ON p.node_id = n.tree_parent_id \
                     WHERE c.depth < $2) \
                     SELECT id, depth FROM chain",
                )
                .bind(member)
                .bind(MAX_DEPTH)
                .fetch_all(&mut *$tx)
            );
            // The space sits above every top-level node, as a reindex writes it.
            let mut chain = dedup_chain(chain);
            let top = chain.iter().map(|(_, depth)| *depth).max().unwrap_or(0);
            chain.push((space, top.saturating_add(1)));
            for (ancestor, depth) in chain {
                let _ = run!(
                    $system,
                    "closure.insert",
                    sqlx::query(
                        "INSERT INTO page_ancestors (space_id, ancestor_id, descendant_id, depth) \
                         VALUES ($1, $2, $3, $4)",
                    )
                    .bind(space)
                    .bind(ancestor)
                    .bind(member)
                    .bind(depth)
                    .execute(&mut *$tx)
                );
            }
        }
    }};
}

/// Writes a node's rows in the shared `folders` and `documents` tables.
macro_rules! project_node {
    ($tx:ident, $system:ident, $node:expr, $now:expr) => {{
        let node: &Node = $node;
        let (id, space) = (uuid(node.id), uuid(node.space));
        let path = node.path.as_str();
        let parent: Option<Uuid> = match node.parent {
            Some(parent) => {
                let found: Option<(Uuid,)> = run!(
                    $system,
                    "folder.parent",
                    sqlx::query_as(
                        "SELECT folder_id FROM folders WHERE space_id = $1 AND folder_id = $2",
                    )
                    .bind(space)
                    .bind(uuid(parent))
                    .fetch_optional(&mut *$tx)
                );
                found.map(|(p,)| p)
            }
            None => None,
        };
        let _ = run!(
            $system,
            "document.clear",
            sqlx::query("DELETE FROM documents WHERE doc_id = $1 OR (space_id = $2 AND path = $3)")
                .bind(id)
                .bind(space)
                .bind(path)
                .execute(&mut *$tx)
        );
        if node.kind == NodeKind::Folder && node.is_live() {
            let existing: Option<(Uuid,)> = run!(
                $system,
                "folder.read",
                sqlx::query_as("SELECT space_id FROM folders WHERE folder_id = $1")
                    .bind(id)
                    .fetch_optional(&mut *$tx)
            );
            let _ = run!(
                $system,
                "folder.clear_path",
                sqlx::query("DELETE FROM folders WHERE space_id = $1 AND path = $2 AND folder_id <> $3")
                    .bind(space)
                    .bind(path)
                    .bind(id)
                    .execute(&mut *$tx)
            );
            if existing.is_some_and(|(s,)| s == space) {
                // An update in place keeps the folders beneath it.
                let _ = run!(
                    $system,
                    "folder.update",
                    sqlx::query(
                        "UPDATE folders SET parent_id = $3, path = $4, name = $5, indexed_at = $6 \
                         WHERE space_id = $1 AND folder_id = $2",
                    )
                    .bind(space)
                    .bind(id)
                    .bind(parent)
                    .bind(path)
                    .bind(&node.title)
                    .bind($now)
                    .execute(&mut *$tx)
                );
            } else {
                let _ = run!(
                    $system,
                    "folder.clear",
                    sqlx::query("DELETE FROM folders WHERE folder_id = $1")
                        .bind(id)
                        .execute(&mut *$tx)
                );
                let _ = run!(
                    $system,
                    "folder.insert",
                    sqlx::query(
                        "INSERT INTO folders (space_id, folder_id, parent_id, path, name, indexed_at) \
                         VALUES ($1, $2, $3, $4, $5, $6)",
                    )
                    .bind(space)
                    .bind(id)
                    .bind(parent)
                    .bind(path)
                    .bind(&node.title)
                    .bind($now)
                    .execute(&mut *$tx)
                );
            }
            // Children written before their folder hang under it now.
            let _ = run!(
                $system,
                "folder.adopt_folders",
                sqlx::query(
                    "UPDATE folders SET parent_id = $2 WHERE space_id = $1 AND folder_id IN \
                     (SELECT node_id FROM drive_nodes WHERE space_id = $1 AND parent_id = $2)",
                )
                .bind(space)
                .bind(id)
                .execute(&mut *$tx)
            );
            let _ = run!(
                $system,
                "folder.adopt_documents",
                sqlx::query(
                    "UPDATE documents SET folder_id = $2 WHERE space_id = $1 AND doc_id IN \
                     (SELECT node_id FROM drive_nodes WHERE space_id = $1 AND parent_id = $2)",
                )
                .bind(space)
                .bind(id)
                .execute(&mut *$tx)
            );
        } else {
            let _ = run!(
                $system,
                "folder.clear",
                sqlx::query("DELETE FROM folders WHERE folder_id = $1")
                    .bind(id)
                    .execute(&mut *$tx)
            );
        }
        if let Some(kind) = document_kind(node.kind) {
            let trash = node.trash.as_ref();
            let _ = run!(
                $system,
                "document.insert",
                sqlx::query(
                    "INSERT INTO documents (space_id, doc_id, folder_id, kind, path, title, \
                     content_hash, byte_size, modified_at, trashed_at, explicitly_trashed, \
                     indexed_at) VALUES ($1, $2, $3, $4, $5, $6, '', $7, $8, $9, $10, $11)",
                )
                .bind(space)
                .bind(id)
                .bind(parent)
                .bind(kind)
                .bind(path)
                .bind(&node.title)
                .bind(size(node.size)?)
                .bind(node.modified.unix_ms())
                .bind(trash.map(|t| t.trashed_at.unix_ms()))
                .bind(trash.is_some_and(|t| t.explicitly_trashed))
                .bind($now)
                .execute(&mut *$tx)
            );
        }
    }};
}

#[async_trait]
impl DriveIndex for SqlIndex {
    async fn put_space(&self, space: &Space) -> DriveResult<()> {
        let id = uuid(space.id);
        let settings = files::to_yaml(&space.settings)?;
        let now = now_ms();
        on_pool!(self, pool, system => {
            let mut tx = pool.begin().await.map_err(db_error)?;
            let _ = run!(
                system,
                "space.upsert",
                sqlx::query(
                    "INSERT INTO drive_spaces (space_id, kind, slug, name, owner_id, root_path, settings) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7) \
                     ON CONFLICT (space_id) DO UPDATE SET kind = excluded.kind, \
                     slug = excluded.slug, name = excluded.name, owner_id = excluded.owner_id, \
                     root_path = excluded.root_path, settings = excluded.settings",
                )
                .bind(id)
                .bind(space_kind(space.kind))
                .bind(&space.slug)
                .bind(&space.name)
                .bind(space.owner.map(uuid))
                .bind(space.root.as_str())
                .bind(&settings)
                .execute(&mut *tx)
            );
            let _ = project_space!(tx, system, id, now);
            // Other replicas reload their space lists (ADR 0038).
            let (seq,): (i64,) = run!(
                system,
                "space.announce",
                sqlx::query_as("UPDATE cluster_feed SET head = head + 1 WHERE id = 1 RETURNING head")
                    .fetch_one(&mut *tx)
            );
            let _ = run!(
                system,
                "space.announce",
                sqlx::query(
                    "INSERT INTO cluster_changes (seq, topic, subject, origin) VALUES ($1, $2, $3, '')"
                )
                .bind(seq)
                .bind(vellum_db::feed::SPACES)
                .bind(space.root.as_str())
                .execute(&mut *tx)
            );
            tx.commit().await.map_err(db_error)?;
            Ok(())
        })
    }

    async fn space(&self, id: SpaceId) -> DriveResult<Option<Space>> {
        let rows: Vec<SpaceRow> = on_pool!(self, pool, system => {
            Ok(run!(
                system,
                "space.get",
                sqlx::query_as(
                    "SELECT space_id, kind, slug, name, owner_id, root_path, settings \
                     FROM drive_spaces WHERE space_id = $1",
                )
                .bind(uuid(id))
                .fetch_all(pool)
            ))
        })?;
        rows.into_iter().next().map(space_from_row).transpose()
    }

    async fn spaces(&self) -> DriveResult<Vec<Space>> {
        let rows: Vec<SpaceRow> = on_pool!(self, pool, system => {
            Ok(run!(
                system,
                "space.list",
                sqlx::query_as(
                    "SELECT space_id, kind, slug, name, owner_id, root_path, settings FROM drive_spaces",
                )
                .fetch_all(pool)
            ))
        })?;
        let mut out = rows
            .into_iter()
            .map(space_from_row)
            .collect::<DriveResult<Vec<_>>>()?;
        out.sort_by_key(|s| s.id);
        Ok(out)
    }

    async fn put_node(&self, node: &Node) -> DriveResult<()> {
        let (id, space) = (uuid(node.id), uuid(node.space));
        let tree_parent = tree_parent(node).map(uuid);
        let trash = node.trash.as_ref().map(files::to_yaml).transpose()?;
        let now = now_ms();
        on_pool!(self, pool, system => {
            let mut tx = pool.begin().await.map_err(db_error)?;
            let old: Option<(Uuid, Option<Uuid>)> = run!(
                system,
                "node.previous",
                sqlx::query_as("SELECT space_id, tree_parent_id FROM drive_nodes WHERE node_id = $1")
                    .bind(id)
                    .fetch_optional(&mut *tx)
            );
            let _ = run!(
                system,
                "node.clear",
                sqlx::query("DELETE FROM drive_nodes WHERE node_id = $1")
                    .bind(id)
                    .execute(&mut *tx)
            );
            let _ = run!(
                system,
                "node.insert",
                sqlx::query(
                    "INSERT INTO drive_nodes (space_id, node_id, kind, parent_id, tree_parent_id, \
                     path, title, byte_size, modified_at, created_by, target_id, trash) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                )
                .bind(space)
                .bind(id)
                .bind(node_kind(node.kind))
                .bind(node.parent.map(uuid))
                .bind(tree_parent)
                .bind(node.path.as_str())
                .bind(&node.title)
                .bind(size(node.size)?)
                .bind(node.modified.unix_ms())
                .bind(node.created_by.map(uuid))
                .bind(node.target.map(uuid))
                .bind(&trash)
                .execute(&mut *tx)
            );
            if old.is_some_and(|(s, _)| s != space) {
                // Rows keyed by the node follow it to its new space.
                let _ = run!(
                    system,
                    "node.move_links",
                    sqlx::query("UPDATE drive_links SET space_id = $2 WHERE source_id = $1")
                        .bind(id)
                        .bind(space)
                        .execute(&mut *tx)
                );
                let _ = run!(
                    system,
                    "node.move_versions",
                    sqlx::query("UPDATE drive_versions SET space_id = $2 WHERE doc_id = $1")
                        .bind(id)
                        .bind(space)
                        .execute(&mut *tx)
                );
                let _ = run!(
                    system,
                    "node.move_user_state",
                    sqlx::query("UPDATE user_doc_state SET space_id = $2 WHERE doc_id = $1")
                        .bind(id)
                        .bind(space)
                        .execute(&mut *tx)
                );
            }
            let spaced: i64 = run!(
                system,
                "node.space_indexed",
                sqlx::query_scalar("SELECT COUNT(*) FROM spaces WHERE space_id = $1")
                    .bind(space)
                    .fetch_one(&mut *tx)
            );
            // The shared tables need the space; it comes back after a reindex.
            if spaced > 0 || project_space!(tx, system, space, now) {
                project_node!(tx, system, node, now);
                let placed: i64 = run!(
                    system,
                    "closure.placed",
                    sqlx::query_scalar(
                        "SELECT COUNT(*) FROM page_ancestors \
                         WHERE space_id = $1 AND descendant_id = $2 AND ancestor_id = $2",
                    )
                    .bind(space)
                    .bind(id)
                    .fetch_one(&mut *tx)
                );
                let moved = old.is_none_or(|(_, parent)| parent != tree_parent);
                if placed == 0 || moved {
                    rebuild_closure!(tx, system, id, space);
                }
            }
            tx.commit().await.map_err(db_error)?;
            Ok(())
        })
    }

    async fn node(&self, id: ObjectId) -> DriveResult<Option<Node>> {
        let row: Option<NodeRow> = on_pool!(self, pool, system => {
            Ok(run!(
                system,
                "node.get",
                sqlx::query_as(select_nodes!("node_id = $1"))
                    .bind(uuid(id))
                    .fetch_optional(pool)
            ))
        })?;
        row.map(node_from_row).transpose()
    }

    async fn node_at(&self, space: SpaceId, path: &BlobPath) -> DriveResult<Option<Node>> {
        let row: Option<NodeRow> = on_pool!(self, pool, system => {
            Ok(run!(
                system,
                "node.at",
                sqlx::query_as(select_nodes!("space_id = $1 AND path = $2"))
                .bind(uuid(space))
                .bind(path.as_str())
                .fetch_optional(pool)
            ))
        })?;
        row.map(node_from_row).transpose()
    }

    async fn children(&self, space: SpaceId, parent: Option<ObjectId>) -> DriveResult<Vec<Node>> {
        let rows: Vec<NodeRow> = on_pool!(self, pool, system => {
            Ok(match parent {
                Some(parent) => run!(
                    system,
                    "node.children",
                    sqlx::query_as(select_nodes!("space_id = $1 AND parent_id = $2 AND trash IS NULL"))
                    .bind(uuid(space))
                    .bind(uuid(parent))
                    .fetch_all(pool)
                ),
                None => run!(
                    system,
                    "node.top_level",
                    sqlx::query_as(select_nodes!("space_id = $1 AND parent_id IS NULL AND trash IS NULL"))
                    .bind(uuid(space))
                    .fetch_all(pool)
                ),
            })
        })?;
        let mut out = nodes_from_rows(rows)?;
        out.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
        Ok(out)
    }

    async fn nodes(&self, space: SpaceId) -> DriveResult<Vec<Node>> {
        let rows: Vec<NodeRow> = on_pool!(self, pool, system => {
            Ok(run!(
                system,
                "node.list",
                sqlx::query_as(select_nodes!("space_id = $1"))
                    .bind(uuid(space))
                    .fetch_all(pool)
            ))
        })?;
        let mut out = nodes_from_rows(rows)?;
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    async fn by_title(&self, space: SpaceId, title: &str) -> DriveResult<Vec<Node>> {
        let rows: Vec<NodeRow> = on_pool!(self, pool, system => {
            Ok(run!(
                system,
                "node.by_title",
                sqlx::query_as(select_nodes!("space_id = $1 AND title = $2 AND trash IS NULL"))
                .bind(uuid(space))
                .bind(title)
                .fetch_all(pool)
            ))
        })?;
        nodes_from_rows(rows)
    }

    async fn remove_node(&self, id: ObjectId) -> DriveResult<()> {
        let id = uuid(id);
        on_pool!(self, pool, system => {
            let mut tx = pool.begin().await.map_err(db_error)?;
            for (op, sql) in [
                ("node.remove", "DELETE FROM drive_nodes WHERE node_id = $1"),
                ("node.remove_links", "DELETE FROM drive_links WHERE source_id = $1"),
                ("node.remove_versions", "DELETE FROM drive_versions WHERE doc_id = $1"),
                ("node.remove_user_state", "DELETE FROM user_doc_state WHERE doc_id = $1"),
                ("node.remove_document", "DELETE FROM documents WHERE doc_id = $1"),
                ("node.remove_folder", "DELETE FROM folders WHERE folder_id = $1"),
                ("node.remove_closure", "DELETE FROM page_ancestors WHERE descendant_id = $1"),
            ] {
                let _ = traced(system, op, sqlx::query(sql).bind(id).execute(&mut *tx))
                    .await
                    .map_err(db_error)?;
            }
            // Whatever hung beneath it is now top level.
            let orphans: Vec<(Uuid, Uuid)> = run!(
                system,
                "node.orphans",
                sqlx::query_as("SELECT node_id, space_id FROM drive_nodes WHERE tree_parent_id = $1")
                    .bind(id)
                    .fetch_all(&mut *tx)
            );
            for (orphan, space) in orphans {
                let spaced: i64 = run!(
                    system,
                    "node.space_indexed",
                    sqlx::query_scalar("SELECT COUNT(*) FROM spaces WHERE space_id = $1")
                        .bind(space)
                        .fetch_one(&mut *tx)
                );
                if spaced > 0 {
                    rebuild_closure!(tx, system, orphan, space);
                }
            }
            tx.commit().await.map_err(db_error)?;
            Ok(())
        })
    }

    async fn set_starred(
        &self,
        user: UserId,
        node: ObjectId,
        at: Option<Timestamp>,
    ) -> DriveResult<()> {
        let space = self.space_of(node).await?;
        on_pool!(self, pool, system => {
            match at {
                Some(at) => {
                    let _ = run!(
                        system,
                        "user.star",
                        sqlx::query(
                            "INSERT INTO user_doc_state (space_id, user_id, doc_id, starred_at) \
                             VALUES ($1, $2, $3, $4) ON CONFLICT (space_id, user_id, doc_id) \
                             DO UPDATE SET starred_at = excluded.starred_at",
                        )
                        .bind(space)
                        .bind(uuid(user))
                        .bind(uuid(node))
                        .bind(at.unix_ms())
                        .execute(pool)
                    );
                }
                None => {
                    let _ = run!(
                        system,
                        "user.unstar",
                        sqlx::query(
                            "UPDATE user_doc_state SET starred_at = NULL \
                             WHERE user_id = $1 AND doc_id = $2",
                        )
                        .bind(uuid(user))
                        .bind(uuid(node))
                        .execute(pool)
                    );
                }
            }
            Ok(())
        })
    }

    async fn starred(&self, user: UserId) -> DriveResult<Vec<ObjectId>> {
        let rows: Vec<(Uuid, i64)> = on_pool!(self, pool, system => {
            Ok(run!(
                system,
                "user.starred",
                sqlx::query_as(
                    "SELECT doc_id, starred_at FROM user_doc_state \
                     WHERE user_id = $1 AND starred_at IS NOT NULL",
                )
                .bind(uuid(user))
                .fetch_all(pool)
            ))
        })?;
        let mut out: Vec<(ObjectId, i64)> =
            rows.into_iter().map(|(id, at)| (object(id), at)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
        Ok(out.into_iter().map(|(id, _)| id).collect())
    }

    async fn touch(&self, user: UserId, node: ObjectId, at: Timestamp) -> DriveResult<()> {
        let space = self.space_of(node).await?;
        on_pool!(self, pool, system => {
            let _ = run!(
                system,
                "user.touch",
                sqlx::query(
                    "INSERT INTO user_doc_state (space_id, user_id, doc_id, last_viewed_at, view_count) \
                     VALUES ($1, $2, $3, $4, 1) ON CONFLICT (space_id, user_id, doc_id) \
                     DO UPDATE SET last_viewed_at = excluded.last_viewed_at, \
                     view_count = user_doc_state.view_count + 1",
                )
                .bind(space)
                .bind(uuid(user))
                .bind(uuid(node))
                .bind(at.unix_ms())
                .execute(pool)
            );
            Ok(())
        })
    }

    async fn recent(&self, user: UserId, limit: usize) -> DriveResult<Vec<(ObjectId, Timestamp)>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows: Vec<(Uuid, i64)> = on_pool!(self, pool, system => {
            Ok(run!(
                system,
                "user.recent",
                sqlx::query_as(
                    "SELECT doc_id, last_viewed_at FROM user_doc_state \
                     WHERE user_id = $1 AND last_viewed_at IS NOT NULL \
                     ORDER BY last_viewed_at DESC, doc_id DESC LIMIT $2",
                )
                .bind(uuid(user))
                .bind(limit)
                .fetch_all(pool)
            ))
        })?;
        rows.into_iter()
            .map(|(id, at)| Ok((object(id), timestamp(at)?)))
            .collect()
    }

    async fn append_activity(&self, entry: &Activity) -> DriveResult<()> {
        let body = files::to_yaml(&ActivityBody {
            actor: entry.actor,
            ancestors: entry.ancestors.clone(),
            kind: entry.kind.clone(),
        })?;
        let (space, event) = (uuid(entry.space), Uuid::from_u128(entry.id.as_u128()));
        let mut scope: Vec<Uuid> = Vec::with_capacity(entry.ancestors.len() + 1);
        let mut seen = HashSet::new();
        for id in std::iter::once(entry.node).chain(entry.ancestors.iter().copied()) {
            if seen.insert(id) {
                scope.push(uuid(id));
            }
        }
        on_pool!(self, pool, system => {
            let mut tx = pool.begin().await.map_err(db_error)?;
            let _ = run!(
                system,
                "activity.insert",
                sqlx::query(
                    "INSERT INTO drive_activity (space_id, event_id, node_id, at, entry) \
                     VALUES ($1, $2, $3, $4, $5)",
                )
                .bind(space)
                .bind(event)
                .bind(uuid(entry.node))
                .bind(entry.at.unix_ms())
                .bind(&body)
                .execute(&mut *tx)
            );
            for object in &scope {
                let _ = run!(
                    system,
                    "activity.scope",
                    sqlx::query(
                        "INSERT INTO drive_activity_scope (space_id, object_id, event_id) \
                         VALUES ($1, $2, $3)",
                    )
                    .bind(space)
                    .bind(*object)
                    .bind(event)
                    .execute(&mut *tx)
                );
            }
            tx.commit().await.map_err(db_error)?;
            Ok(())
        })
    }

    async fn activity(&self, node: ObjectId, limit: usize) -> DriveResult<Vec<Activity>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows: Vec<ActivityRow> = on_pool!(self, pool, system => {
            Ok(run!(
                system,
                "activity.list",
                sqlx::query_as(
                    "SELECT a.space_id, a.event_id, a.node_id, a.at, a.entry \
                     FROM drive_activity_scope s JOIN drive_activity a \
                     ON a.space_id = s.space_id AND a.event_id = s.event_id \
                     WHERE s.object_id = $1 ORDER BY s.event_id DESC LIMIT $2",
                )
                .bind(uuid(node))
                .bind(limit)
                .fetch_all(pool)
            ))
        })?;
        rows.into_iter().map(activity_from_row).collect()
    }

    async fn set_links(&self, source: ObjectId, targets: &[ObjectId]) -> DriveResult<()> {
        let space = self.space_of(source).await?;
        let mut unique: Vec<ObjectId> = targets.to_vec();
        unique.sort_unstable();
        unique.dedup();
        on_pool!(self, pool, system => {
            let mut tx = pool.begin().await.map_err(db_error)?;
            let _ = run!(
                system,
                "links.clear",
                sqlx::query("DELETE FROM drive_links WHERE source_id = $1")
                    .bind(uuid(source))
                    .execute(&mut *tx)
            );
            for target in &unique {
                let _ = run!(
                    system,
                    "links.insert",
                    sqlx::query(
                        "INSERT INTO drive_links (space_id, source_id, target_id) VALUES ($1, $2, $3)",
                    )
                    .bind(space)
                    .bind(uuid(source))
                    .bind(uuid(*target))
                    .execute(&mut *tx)
                );
            }
            tx.commit().await.map_err(db_error)?;
            Ok(())
        })
    }

    async fn backlinks(&self, target: ObjectId) -> DriveResult<Vec<ObjectId>> {
        let rows: Vec<(Uuid,)> = on_pool!(self, pool, system => {
            Ok(run!(
                system,
                "links.backlinks",
                sqlx::query_as(
                    "SELECT DISTINCT source_id FROM drive_links WHERE target_id = $1 AND source_id <> $1",
                )
                .bind(uuid(target))
                .fetch_all(pool)
            ))
        })?;
        let mut out: Vec<ObjectId> = rows.into_iter().map(|(id,)| object(id)).collect();
        out.sort_unstable();
        Ok(out)
    }

    async fn put_version(&self, version: &VersionRecord) -> DriveResult<()> {
        let author = files::to_yaml(&version.author)?;
        let id = Uuid::from_u128(version.id.as_u128());
        on_pool!(self, pool, system => {
            let mut tx = pool.begin().await.map_err(db_error)?;
            let _ = run!(
                system,
                "version.clear",
                sqlx::query("DELETE FROM drive_versions WHERE version_id = $1")
                    .bind(id)
                    .execute(&mut *tx)
            );
            let _ = run!(
                system,
                "version.insert",
                sqlx::query(
                    "INSERT INTO drive_versions (space_id, version_id, doc_id, byte_size, author) \
                     VALUES ($1, $2, $3, $4, $5)",
                )
                .bind(uuid(version.space))
                .bind(id)
                .bind(uuid(version.doc))
                .bind(size(version.size)?)
                .bind(&author)
                .execute(&mut *tx)
            );
            tx.commit().await.map_err(db_error)?;
            Ok(())
        })
    }

    async fn versions(&self, doc: ObjectId) -> DriveResult<Vec<VersionRecord>> {
        let rows: Vec<(Uuid, Uuid, Uuid, i64, String)> = on_pool!(self, pool, system => {
            Ok(run!(
                system,
                "version.list",
                sqlx::query_as(
                    "SELECT version_id, doc_id, space_id, byte_size, author FROM drive_versions \
                     WHERE doc_id = $1 ORDER BY version_id DESC",
                )
                .bind(uuid(doc))
                .fetch_all(pool)
            ))
        })?;
        rows.into_iter()
            .map(|(id, doc, space, bytes, author)| {
                Ok(VersionRecord {
                    id: VersionId::from_u128(id.as_u128()),
                    doc: object(doc),
                    space: SpaceId::from_u128(space.as_u128()),
                    size: unsigned(bytes),
                    author: from_yaml(&author)?,
                })
            })
            .collect()
    }

    async fn remove_version(&self, id: VersionId) -> DriveResult<()> {
        on_pool!(self, pool, system => {
            let _ = run!(
                system,
                "version.remove",
                sqlx::query("DELETE FROM drive_versions WHERE version_id = $1")
                    .bind(Uuid::from_u128(id.as_u128()))
                    .execute(pool)
            );
            Ok(())
        })
    }

    async fn usage(&self, space: SpaceId) -> DriveResult<u64> {
        let (nodes, versions): (i64, i64) = on_pool!(self, pool, system => {
            let nodes: i64 = run!(
                system,
                "usage.nodes",
                sqlx::query_scalar(
                    "SELECT CAST(COALESCE(SUM(byte_size), 0) AS BIGINT) FROM drive_nodes \
                     WHERE space_id = $1",
                )
                .bind(uuid(space))
                .fetch_one(pool)
            );
            let versions: i64 = run!(
                system,
                "usage.versions",
                sqlx::query_scalar(
                    "SELECT CAST(COALESCE(SUM(byte_size), 0) AS BIGINT) FROM drive_versions \
                     WHERE space_id = $1",
                )
                .bind(uuid(space))
                .fetch_one(pool)
            );
            Ok((nodes, versions))
        })?;
        Ok(unsigned(nodes).saturating_add(unsigned(versions)))
    }

    async fn totals(&self, now: Timestamp) -> DriveResult<DriveTotals> {
        const DAY_MS: i64 = 86_400_000;
        type Counts = (i64, Vec<(String, i64, i64, i64)>, (i64, i64), i64, i64);
        let day = now.unix_ms().saturating_sub(DAY_MS);
        let week = now.unix_ms().saturating_sub(7 * DAY_MS);
        let (spaces, nodes, versions, events_1d, events_7d): Counts = on_pool!(self, pool, system => {
            let spaces: i64 = run!(
                system,
                "totals.spaces",
                sqlx::query_scalar("SELECT COUNT(*) FROM drive_spaces").fetch_one(pool)
            );
            let nodes: Vec<(String, i64, i64, i64)> = run!(
                system,
                "totals.nodes",
                sqlx::query_as(
                    "SELECT kind, CASE WHEN trash IS NULL THEN 1 ELSE 0 END AS live, \
                     COUNT(*), CAST(COALESCE(SUM(byte_size), 0) AS BIGINT) \
                     FROM drive_nodes GROUP BY kind, CASE WHEN trash IS NULL THEN 1 ELSE 0 END",
                )
                .fetch_all(pool)
            );
            let versions: (i64, i64) = run!(
                system,
                "totals.versions",
                sqlx::query_as(
                    "SELECT COUNT(*), CAST(COALESCE(SUM(byte_size), 0) AS BIGINT) \
                     FROM drive_versions",
                )
                .fetch_one(pool)
            );
            let events: (i64, i64) = run!(
                system,
                "totals.activity",
                sqlx::query_as(
                    "SELECT CAST(COALESCE(SUM(CASE WHEN at >= $1 THEN 1 ELSE 0 END), 0) AS BIGINT), \
                     COUNT(*) FROM drive_activity WHERE at >= $2",
                )
                .bind(day)
                .bind(week)
                .fetch_one(pool)
            );
            Ok((spaces, nodes, versions, events.0, events.1))
        })?;
        let mut totals = DriveTotals {
            spaces: unsigned(spaces),
            versions: unsigned(versions.0),
            version_bytes: unsigned(versions.1),
            events_1d: unsigned(events_1d),
            events_7d: unsigned(events_7d),
            ..DriveTotals::default()
        };
        for (kind, live, n, bytes) in nodes {
            let kind = parse_node_kind(&kind)?;
            totals.add_nodes(kind, live == 1, unsigned(n));
            totals.file_bytes = totals.file_bytes.saturating_add(unsigned(bytes));
        }
        Ok(totals)
    }
}

impl SqlIndex {
    /// The space a node lives in, or the nil ID for a node not indexed.
    async fn space_of(&self, node: ObjectId) -> DriveResult<Uuid> {
        let row: Option<(Uuid,)> = on_pool!(self, pool, system => {
            Ok(run!(
                system,
                "node.space",
                sqlx::query_as("SELECT space_id FROM drive_nodes WHERE node_id = $1")
                    .bind(uuid(node))
                    .fetch_optional(pool)
            ))
        })?;
        Ok(row.map_or(Uuid::nil(), |(space,)| space))
    }
}

type SpaceRow = (Uuid, String, String, String, Option<Uuid>, String, String);

type NodeRow = (
    Uuid,
    Uuid,
    String,
    Option<Uuid>,
    String,
    String,
    i64,
    i64,
    Option<Uuid>,
    Option<Uuid>,
    Option<String>,
);

type ActivityRow = (Uuid, Uuid, Uuid, i64, String);

/// The parts of an activity entry stored as YAML.
#[derive(Serialize, Deserialize)]
struct ActivityBody {
    actor: Actor,
    ancestors: Vec<ObjectId>,
    kind: ActivityKind,
}

fn space_from_row(row: SpaceRow) -> DriveResult<Space> {
    let (id, kind, slug, name, owner, root, settings) = row;
    let kind = match kind.as_str() {
        "shared_drive" => SpaceKind::SharedDrive,
        "user_drive" => SpaceKind::UserDrive,
        other => return Err(DriveError::Index(format!("unknown space kind {other}"))),
    };
    let settings: SpaceSettings = from_yaml(&settings)?;
    Ok(Space {
        id: SpaceId::from_u128(id.as_u128()),
        kind,
        slug,
        name,
        owner: owner.map(|o| UserId::from_u128(o.as_u128())),
        settings,
        root: blob_path(&root)?,
    })
}

fn node_from_row(row: NodeRow) -> DriveResult<Node> {
    let (space, id, kind, parent, path, title, bytes, modified, created_by, target, trash) = row;
    let trash: Option<TrashState> = trash.map(|t| from_yaml(&t)).transpose()?;
    Ok(Node {
        id: object(id),
        space: SpaceId::from_u128(space.as_u128()),
        kind: parse_node_kind(&kind)?,
        parent: parent.map(object),
        path: blob_path(&path)?,
        title,
        size: unsigned(bytes),
        modified: timestamp(modified)?,
        created_by: created_by.map(|u| UserId::from_u128(u.as_u128())),
        target: target.map(object),
        trash,
    })
}

fn nodes_from_rows(rows: Vec<NodeRow>) -> DriveResult<Vec<Node>> {
    rows.into_iter().map(node_from_row).collect()
}

fn activity_from_row(row: ActivityRow) -> DriveResult<Activity> {
    let (space, event, node, at, body) = row;
    let body: ActivityBody = from_yaml(&body)?;
    Ok(Activity {
        id: EventId::from_u128(event.as_u128()),
        at: timestamp(at)?,
        actor: body.actor,
        space: SpaceId::from_u128(space.as_u128()),
        node: object(node),
        ancestors: body.ancestors,
        kind: body.kind,
    })
}

/// The parent a node's access inherits from (see [`crate::authz`]).
const fn tree_parent(node: &Node) -> Option<ObjectId> {
    match &node.trash {
        Some(trash) => trash.original_parent,
        None => node.parent,
    }
}

/// Keeps the shallowest depth for each ancestor, dropping cycle repeats.
fn dedup_chain(chain: Vec<(Uuid, i32)>) -> Vec<(Uuid, i32)> {
    let mut seen = HashSet::new();
    let mut out: Vec<(Uuid, i32)> = chain;
    out.sort_by_key(|(_, depth)| *depth);
    out.retain(|(id, _)| seen.insert(*id));
    out
}

const fn space_kind(kind: SpaceKind) -> &'static str {
    match kind {
        SpaceKind::SharedDrive => "shared_drive",
        SpaceKind::UserDrive => "user_drive",
    }
}

const fn node_kind(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Folder => "folder",
        NodeKind::Page => "page",
        NodeKind::Book => "book",
        NodeKind::Collection => "collection",
        NodeKind::Shortcut => "shortcut",
    }
}

/// The `documents.kind` of a node; folders live in `folders` instead.
const fn document_kind(kind: NodeKind) -> Option<&'static str> {
    match kind {
        NodeKind::Folder => None,
        other => Some(node_kind(other)),
    }
}

fn parse_node_kind(kind: &str) -> DriveResult<NodeKind> {
    Ok(match kind {
        "folder" => NodeKind::Folder,
        "page" => NodeKind::Page,
        "book" => NodeKind::Book,
        "collection" => NodeKind::Collection,
        "shortcut" => NodeKind::Shortcut,
        other => return Err(DriveError::Index(format!("unknown node kind {other}"))),
    })
}

fn uuid(id: impl Into<ObjectId>) -> Uuid {
    Uuid::from_u128(id.into().as_u128())
}

const fn object(id: Uuid) -> ObjectId {
    ObjectId::from_u128(id.as_u128())
}

fn blob_path(path: &str) -> DriveResult<BlobPath> {
    BlobPath::new(path).map_err(|err| DriveError::Index(format!("stored path {path}: {err}")))
}

fn timestamp(ms: i64) -> DriveResult<Timestamp> {
    Timestamp::from_unix_ms(ms).map_err(|err| DriveError::Index(err.to_string()))
}

fn size(bytes: u64) -> DriveResult<i64> {
    i64::try_from(bytes).map_err(|_| DriveError::Index(format!("size {bytes} is too large")))
}

fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn from_yaml<T: serde::de::DeserializeOwned>(text: &str) -> DriveResult<T> {
    serde_saphyr::from_str(text).map_err(|err| DriveError::Index(format!("stored YAML: {err}")))
}

fn db_error(err: impl fmt::Display) -> DriveError {
    DriveError::Index(err.to_string())
}
