// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Page content and version history.

use bytes::Bytes;
use vellum_kernel::{Actor, ObjectId, SpaceId, VersionId};
use vellum_store::{Base, Blob, BlobPath, Precondition, Writer};

use super::Drive;
use crate::error::{DriveError, DriveResult};
use crate::files::{VersionFile, from_yaml, to_yaml};
use crate::frontmatter;
use crate::layout;
use crate::links::{self, Target};
use crate::model::{ActivityKind, Node, NodeKind, VersionRecord};
use crate::ports::Permission;
use crate::retention;

/// How a save landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    /// Nobody else had changed the page; it holds the writer's content.
    Written(VersionId),
    /// Concurrent changes were merged with the writer's.
    Merged(VersionId),
    /// The changes conflicted: the stored page is unchanged and the
    /// writer's content went to a new conflict-copy page.
    Conflicted {
        /// The conflict copy.
        copy: Box<Node>,
    },
}

/// The result of [`Drive::save`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveOutcome {
    /// The page after the save (renamed if its title changed).
    pub node: Node,
    /// How the save landed.
    pub saved: Saved,
}

impl Drive {
    /// Opens a page (or any file node) and records it in the reader's
    /// recents.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`, `Invalid` for folders.
    #[tracing::instrument(name = "drive.read", skip_all, fields(node = %id), err)]
    pub async fn read(&self, actor: &Actor, id: ObjectId) -> DriveResult<(Node, Blob)> {
        self.observe("read", async {
            let node = self.live_node(id).await?;
            self.authorize(actor, node.space, node.id, Permission::View)
                .await?;
            let path = layout::id_file(node.kind, &node.path)?;
            let blob = self.store.get(&path).await?;
            if let Some(user) = Self::user_of(actor) {
                self.index.touch(user, node.id, self.clock.now()).await?;
            }
            Ok((node, blob))
        })
        .await
    }

    /// Saves new content for a page. `base` is the content and etag the
    /// writer started from: concurrent changes are merged three-way, and
    /// on conflict the writer's content goes to a conflict copy, never
    /// over someone else's (D12). A changed front-matter title renames the
    /// file and rewrites links to it.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`, `Invalid` for non-pages,
    /// `QuotaExceeded`, or a store failure.
    #[tracing::instrument(name = "drive.save", skip_all, fields(node = %id), err)]
    pub async fn save(
        &self,
        actor: &Actor,
        id: ObjectId,
        base: Base,
        content: Bytes,
    ) -> DriveResult<SaveOutcome> {
        self.observe("save", async {
            let node = self.live_node(id).await?;
            if node.kind != NodeKind::Page {
                return Err(DriveError::Invalid(
                    "only pages have editable content".into(),
                ));
            }
            self.authorize(actor, node.space, node.id, Permission::Edit)
                .await?;
            let space = self.space(node.space).await?;
            let size = content.len() as u64;
            self.ensure_quota(&space, size.saturating_sub(node.size).saturating_add(size))
                .await?;
            self.store_save(actor, node, base, content).await
        })
        .await
    }

    /// Writes through the store's merge path and updates everything that
    /// follows from new content.
    async fn store_save(
        &self,
        actor: &Actor,
        mut node: Node,
        base: Base,
        content: Bytes,
    ) -> DriveResult<SaveOutcome> {
        let who = writer_name(actor);
        let date = self.clock.now().to_string();
        let reidentify = |bytes: &[u8]| {
            Bytes::from(frontmatter::set_md(
                &String::from_utf8_lossy(bytes),
                "id",
                &ObjectId::generate().to_string(),
            ))
        };
        let writer = Writer {
            user: &who,
            date: date.get(..10).unwrap_or(&date),
            reidentify: &reidentify,
        };
        let saved = vellum_store::save(
            self.store.as_ref(),
            &node.path,
            base,
            content.clone(),
            &writer,
        )
        .await?;
        let (meta, merged) = match saved {
            vellum_store::Saved::Written(meta) => (meta, false),
            vellum_store::Saved::Merged(meta) => (meta, true),
            vellum_store::Saved::Conflicted { copy, .. } => {
                let copy = self.index_conflict_copy(actor, &node, &copy.path).await?;
                return Ok(SaveOutcome {
                    node,
                    saved: Saved::Conflicted {
                        copy: Box::new(copy),
                    },
                });
            }
        };
        let bytes = if merged {
            self.store.get(&node.path).await?.bytes
        } else {
            content
        };
        node.size = meta.size;
        node.modified = self.clock.now();
        let version = self.write_version(actor, &node, &bytes).await?;
        self.index.put_node(&node).await?;
        let text = String::from_utf8_lossy(&bytes).into_owned();
        self.reindex_links(&node, &text).await?;
        self.record(
            actor,
            &node,
            ActivityKind::Edited {
                version: version.id,
            },
        )
        .await?;
        if let Some(user) = Self::user_of(actor) {
            self.index.touch(user, node.id, node.modified).await?;
        }
        if let Some(title) = frontmatter::get_md(&text, "title") {
            let title = title.trim();
            if !title.is_empty() && title != node.title {
                node = self.retitle(actor, node, title).await?;
            }
        }
        Ok(SaveOutcome {
            node,
            saved: if merged {
                Saved::Merged(version.id)
            } else {
                Saved::Written(version.id)
            },
        })
    }

    /// Indexes a conflict copy written next to `original`.
    async fn index_conflict_copy(
        &self,
        actor: &Actor,
        original: &Node,
        path: &vellum_store::BlobPath,
    ) -> DriveResult<Node> {
        let (text, blob) = self.read_text(path).await?;
        let id = frontmatter::get_md(&text, "id")
            .and_then(|id| id.parse::<ObjectId>().ok())
            .ok_or_else(|| DriveError::Malformed {
                path: path.to_string(),
                reason: "conflict copy has no id".into(),
            })?;
        let copy = Node {
            id,
            space: original.space,
            kind: NodeKind::Page,
            parent: original.parent,
            path: path.clone(),
            title: layout::title_from_path(NodeKind::Page, path),
            size: blob.meta.size,
            modified: self.clock.now(),
            created_by: Self::user_of(actor),
            target: None,
            trash: None,
        };
        self.created(actor, &copy).await?;
        let _ = self.write_version(actor, &copy, &blob.bytes).await?;
        self.reindex_links(&copy, &text).await?;
        self.record(
            actor,
            original,
            ActivityKind::ConflictCopy { copy: copy.id },
        )
        .await?;
        Ok(copy)
    }

    /// Stores a version of `node`'s content and indexes it.
    pub(super) async fn write_version(
        &self,
        actor: &Actor,
        node: &Node,
        bytes: &Bytes,
    ) -> DriveResult<VersionRecord> {
        let record = VersionRecord {
            id: self.version_id(),
            doc: node.id,
            space: node.space,
            size: bytes.len() as u64,
            author: *actor,
        };
        let (content, sidecar) = layout::version_paths(node.id, record.id)?;
        let _ = self
            .store
            .put(&content, bytes.clone(), Precondition::IfAbsent)
            .await?;
        let meta = VersionFile {
            space: node.space,
            author: *actor,
        };
        let _ = self.put_file(&sidecar, to_yaml(&meta)?).await?;
        self.index.put_version(&record).await?;
        Ok(record)
    }

    /// A fresh version ID stamped with the drive's clock, so retention
    /// ages versions by the same clock that decides "now".
    fn version_id(&self) -> VersionId {
        let ms = u128::from(u64::try_from(self.clock.now().unix_ms()).unwrap_or(0));
        let random = VersionId::generate().as_u128() & ((1_u128 << 80) - 1);
        VersionId::from_u128((ms << 80) | random)
    }

    /// A page's versions, newest first.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`.
    #[tracing::instrument(name = "drive.versions", skip_all, fields(node = %id), err)]
    pub async fn versions(&self, actor: &Actor, id: ObjectId) -> DriveResult<Vec<VersionRecord>> {
        self.observe("versions", async {
            let node = self.live_node(id).await?;
            self.authorize(actor, node.space, node.id, Permission::View)
                .await?;
            self.index.versions(node.id).await
        })
        .await
    }

    /// The content of one version.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`.
    #[tracing::instrument(name = "drive.version_content", skip_all, fields(node = %id), err)]
    pub async fn version_content(
        &self,
        actor: &Actor,
        id: ObjectId,
        version: VersionId,
    ) -> DriveResult<Bytes> {
        self.observe("version_content", async {
            let node = self.live_node(id).await?;
            self.authorize(actor, node.space, node.id, Permission::View)
                .await?;
            let (content, _) = layout::version_paths(node.id, version)?;
            match self.store.get(&content).await {
                Ok(blob) => Ok(blob.bytes),
                Err(vellum_store::StoreError::NotFound(_)) => Err(DriveError::NotFound(id)),
                Err(err) => Err(err.into()),
            }
        })
        .await
    }

    /// Makes an earlier version the current content, as a new version.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`, `QuotaExceeded`.
    #[tracing::instrument(name = "drive.restore_version", skip_all, fields(node = %id), err)]
    pub async fn restore_version(
        &self,
        actor: &Actor,
        id: ObjectId,
        version: VersionId,
    ) -> DriveResult<SaveOutcome> {
        self.observe("restore_version", async {
            let old = self.version_content(actor, id, version).await?;
            let current = self.store.get(&self.live_node(id).await?.path).await?;
            let base = Base {
                bytes: current.bytes,
                etag: Some(current.meta.etag),
            };
            let outcome = Box::pin(self.save(actor, id, base, old)).await?;
            self.record(
                actor,
                &outcome.node,
                ActivityKind::VersionRestored { version },
            )
            .await?;
            Ok(outcome)
        })
        .await
    }

    /// Deletes the versions a space's retention policy no longer keeps.
    /// Returns how many went.
    ///
    /// # Errors
    /// `SpaceNotFound`, `PermissionDenied` for non-managers, store failures.
    #[tracing::instrument(name = "drive.prune_versions", skip_all, fields(space = %space), err)]
    pub async fn prune_versions(&self, actor: &Actor, space: SpaceId) -> DriveResult<usize> {
        self.observe("prune_versions", async {
            let space = self.space(space).await?;
            self.authorize(actor, space.id, space.node_id(), Permission::Manage)
                .await?;
            let now = u64::try_from(self.clock.now().unix_ms()).unwrap_or(0);
            let mut pruned = 0;
            for node in self.index.nodes(space.id).await? {
                if node.kind != NodeKind::Page {
                    continue;
                }
                let versions = self.index.versions(node.id).await?;
                for version in retention::prunable(&versions, &space.settings.retention, now) {
                    self.delete_version(node.id, version).await?;
                    pruned += 1;
                }
            }
            Ok(pruned)
        })
        .await
    }

    /// Removes one version's files and index row.
    pub(super) async fn delete_version(
        &self,
        doc: ObjectId,
        version: VersionId,
    ) -> DriveResult<()> {
        let (content, sidecar) = layout::version_paths(doc, version)?;
        for path in <[BlobPath; 2]>::from((content, sidecar)) {
            match self.store.delete(&path, Precondition::None).await {
                Ok(()) | Err(vellum_store::StoreError::NotFound(_)) => {}
                Err(err) => return Err(err.into()),
            }
        }
        self.index.remove_version(version).await
    }

    /// Reads a version's sidecar during a rebuild.
    pub(super) async fn read_version_file(
        &self,
        path: &vellum_store::BlobPath,
    ) -> DriveResult<VersionFile> {
        let blob = self.store.get(path).await?;
        from_yaml(path, &blob.bytes)
    }

    /// Recomputes which nodes a page links to.
    pub(super) async fn reindex_links(&self, node: &Node, text: &str) -> DriveResult<()> {
        let dir = node.path.parent();
        let mut targets = Vec::new();
        for link in links::scan(text) {
            match link.target {
                Target::Id(id) => targets.push(id),
                Target::Wiki(title) => {
                    let mut found = self.index.by_title(node.space, &title).await?;
                    found.sort_by_key(|n| n.id);
                    if let Some(first) = found.first() {
                        targets.push(first.id);
                    }
                }
                Target::Relative(rel) => {
                    if let Some(path) = links::resolve(dir.as_ref(), &rel)
                        && let Some(found) = self.index.node_at(node.space, &path).await?
                    {
                        targets.push(found.id);
                    }
                }
            }
        }
        targets.retain(|t| *t != node.id);
        self.index.set_links(node.id, &targets).await
    }

    /// Applies `edit` to a page's text and saves it through the merge
    /// path if it changed. Used to rewrite links after renames and moves.
    pub(super) async fn rewrite_page(
        &self,
        actor: &Actor,
        node: Node,
        edit: impl FnOnce(&str) -> Option<String> + Send,
    ) -> DriveResult<Node> {
        let (text, blob) = self.read_text(&node.path).await?;
        let Some(new) = edit(&text).filter(|new| *new != text) else {
            return Ok(node);
        };
        let base = Base {
            bytes: blob.bytes,
            etag: Some(blob.meta.etag),
        };
        Ok(
            Box::pin(self.store_save(actor, node, base, Bytes::from(new)))
                .await?
                .node,
        )
    }
}

/// The name a conflict copy carries for the writer.
fn writer_name(actor: &Actor) -> String {
    match actor {
        Actor::User { user } => user.to_string(),
        Actor::Model { on_behalf_of } => format!("{on_behalf_of} via model"),
        Actor::External => "external edit".to_owned(),
        _ => "vellum".to_owned(),
    }
}
