// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Creating spaces, folders, pages and shortcuts.

use bytes::Bytes;
use vellum_kernel::{Actor, ObjectId, SpaceId, UserId};
use vellum_store::Precondition;

use super::Drive;
use crate::error::{DriveError, DriveResult};
use crate::files::{FolderFile, ShortcutFile, SpaceFile, to_yaml};
use crate::frontmatter;
use crate::layout;
use crate::model::{ActivityKind, Node, NodeKind, Space, SpaceKind, SpaceSettings};
use crate::ports::Permission;

impl Drive {
    /// Creates a shared drive (`owner: None`) or a person's My Drive.
    ///
    /// The creator of a shared drive becomes its manager; a My Drive
    /// belongs to its owner, who is the only one who may create it besides
    /// Vellum itself.
    ///
    /// # Errors
    /// `Invalid` for a bad slug or a My Drive without an owner,
    /// `AlreadyExists` if the slug is taken, `PermissionDenied`.
    #[tracing::instrument(name = "drive.create_space", skip_all, fields(slug = slug), err)]
    pub async fn create_space(
        &self,
        actor: &Actor,
        kind: SpaceKind,
        slug: &str,
        name: &str,
        owner: Option<UserId>,
    ) -> DriveResult<Space> {
        self.observe("create_space", async {
            let creator = Self::user_of(actor);
            match (kind, owner, actor) {
                (SpaceKind::UserDrive, None, _) => {
                    return Err(DriveError::Invalid("a My Drive needs an owner".into()));
                }
                (SpaceKind::UserDrive, Some(owner), Actor::User { user }) if owner != *user => {
                    return Err(DriveError::PermissionDenied);
                }
                (_, _, Actor::Model { .. }) => return Err(DriveError::PermissionDenied),
                _ => {}
            }
            let root = layout::space_root(kind, slug)?;
            let file = root.join(layout::FOLDER_FILE)?;
            let space = Space {
                id: SpaceId::generate(),
                kind,
                slug: slug.to_owned(),
                name: name.trim().to_owned(),
                owner,
                settings: SpaceSettings::default(),
                root,
            };
            let text = to_yaml(&space_file(&space))?;
            match self
                .store
                .put(&file, Bytes::from(text), Precondition::IfAbsent)
                .await
            {
                Ok(_) => {}
                Err(vellum_store::StoreError::AlreadyExists(_)) => {
                    return Err(DriveError::AlreadyExists(slug.to_owned()));
                }
                Err(err) => return Err(err.into()),
            }
            self.index.put_space(&space).await?;
            let manager = owner.or(creator);
            if let Some(user) = manager {
                self.access
                    .grant_owner(user, space.id, space.node_id())
                    .await?;
            }
            self.access
                .tree_changed(space.id, &[space.node_id()])
                .await?;
            Ok(space)
        })
        .await
    }

    /// Changes a space's display name and settings (quota, trash days,
    /// version retention). Managers only.
    ///
    /// # Errors
    /// `SpaceNotFound`, `PermissionDenied`, or a store failure.
    #[tracing::instrument(name = "drive.update_space", skip_all, fields(space = %space), err)]
    pub async fn update_space(
        &self,
        actor: &Actor,
        space: SpaceId,
        name: Option<&str>,
        settings: SpaceSettings,
    ) -> DriveResult<Space> {
        self.observe("update_space", async {
            let mut space = self.space(space).await?;
            self.authorize(actor, space.id, space.node_id(), Permission::Manage)
                .await?;
            if let Some(name) = name {
                space.name = name.trim().to_owned();
            }
            space.settings = settings;
            let file = space.root.join(layout::FOLDER_FILE)?;
            let _ = self.put_file(&file, to_yaml(&space_file(&space))?).await?;
            self.index.put_space(&space).await?;
            Ok(space)
        })
        .await
    }

    /// Creates a folder in `parent` (`None`: the space root).
    ///
    /// # Errors
    /// `PermissionDenied` without edit rights on the parent, `Invalid` if
    /// the parent is not a folder of the space.
    #[tracing::instrument(name = "drive.create_folder", skip_all, fields(space = %space), err)]
    pub async fn create_folder(
        &self,
        actor: &Actor,
        space: SpaceId,
        parent: Option<ObjectId>,
        name: &str,
    ) -> DriveResult<Node> {
        self.observe("create_folder", async {
            let space = self.space(space).await?;
            self.authorize(
                actor,
                space.id,
                parent.unwrap_or_else(|| space.node_id()),
                Permission::Edit,
            )
            .await?;
            let dir = self.dir_of(&space, parent).await?;
            let path = self.free_path(&dir, NodeKind::Folder, name, None).await?;
            let id = ObjectId::generate();
            let file = FolderFile {
                id,
                icon: None,
                space: None,
            };
            let _ = self
                .store
                .put(
                    &path.join(layout::FOLDER_FILE)?,
                    Bytes::from(to_yaml(&file)?),
                    Precondition::IfAbsent,
                )
                .await?;
            let node = Node {
                id,
                space: space.id,
                kind: NodeKind::Folder,
                parent,
                title: layout::title_from_path(NodeKind::Folder, &path),
                path,
                size: 0,
                modified: self.clock.now(),
                created_by: Self::user_of(actor),
                target: None,
                trash: None,
            };
            self.created(actor, &node).await?;
            Ok(node)
        })
        .await
    }

    /// Creates a page titled `title` with Markdown `body` (no front
    /// matter; the drive writes `id`, `title` and `vellum: 1`).
    ///
    /// # Errors
    /// `PermissionDenied`, `Invalid`, or `QuotaExceeded`.
    #[tracing::instrument(name = "drive.create_page", skip_all, fields(space = %space), err)]
    pub async fn create_page(
        &self,
        actor: &Actor,
        space: SpaceId,
        parent: Option<ObjectId>,
        title: &str,
        body: &str,
    ) -> DriveResult<Node> {
        self.observe("create_page", async {
            let space = self.space(space).await?;
            self.authorize(
                actor,
                space.id,
                parent.unwrap_or_else(|| space.node_id()),
                Permission::Edit,
            )
            .await?;
            let dir = self.dir_of(&space, parent).await?;
            let title = title.trim();
            let path = self.free_path(&dir, NodeKind::Page, title, None).await?;
            let id = ObjectId::generate();
            let mut yaml = frontmatter::set("", "id", &id.to_string());
            yaml = frontmatter::set(&yaml, "title", title);
            let text = format!("---\n{yaml}vellum: 1\n---\n\n{body}");
            let bytes = Bytes::from(text);
            let size = bytes.len() as u64;
            // Content plus its first version.
            self.ensure_quota(&space, size.saturating_mul(2)).await?;
            let _ = self
                .store
                .put(&path, bytes.clone(), Precondition::IfAbsent)
                .await?;
            let node = Node {
                id,
                space: space.id,
                kind: NodeKind::Page,
                parent,
                title: title.to_owned(),
                path,
                size,
                modified: self.clock.now(),
                created_by: Self::user_of(actor),
                target: None,
                trash: None,
            };
            self.created(actor, &node).await?;
            let _ = self.write_version(actor, &node, &bytes).await?;
            self.reindex_links(&node, &String::from_utf8_lossy(&bytes))
                .await?;
            Ok(node)
        })
        .await
    }

    /// Creates a shortcut to `target` in `parent`. A shortcut needs view
    /// rights on the target to create, and never grants access to it.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`, `Invalid`.
    #[tracing::instrument(name = "drive.create_shortcut", skip_all, fields(target = %target), err)]
    pub async fn create_shortcut(
        &self,
        actor: &Actor,
        target: ObjectId,
        space: SpaceId,
        parent: Option<ObjectId>,
    ) -> DriveResult<Node> {
        self.observe("create_shortcut", async {
            let target = self.live_node(target).await?;
            if target.kind == NodeKind::Shortcut {
                return Err(DriveError::Invalid(
                    "a shortcut cannot point at a shortcut".into(),
                ));
            }
            self.authorize(actor, target.space, target.id, Permission::View)
                .await?;
            let space = self.space(space).await?;
            self.authorize(
                actor,
                space.id,
                parent.unwrap_or_else(|| space.node_id()),
                Permission::Edit,
            )
            .await?;
            let dir = self.dir_of(&space, parent).await?;
            let path = self
                .free_path(&dir, NodeKind::Shortcut, &target.title, None)
                .await?;
            let file = ShortcutFile {
                id: ObjectId::generate(),
                target: target.id,
                title: target.title.clone(),
            };
            let size = self.put_file(&path, to_yaml(&file)?).await?;
            let node = Node {
                id: file.id,
                space: space.id,
                kind: NodeKind::Shortcut,
                parent,
                title: file.title,
                path,
                size,
                modified: self.clock.now(),
                created_by: Self::user_of(actor),
                target: Some(target.id),
                trash: None,
            };
            self.created(actor, &node).await?;
            Ok(node)
        })
        .await
    }

    /// Indexes a new node, makes its creator its owner and records it.
    pub(super) async fn created(&self, actor: &Actor, node: &Node) -> DriveResult<()> {
        self.index.put_node(node).await?;
        if let Some(user) = Self::user_of(actor) {
            self.access.grant_owner(user, node.space, node.id).await?;
            self.index.touch(user, node.id, node.modified).await?;
        }
        self.access.tree_changed(node.space, &[node.id]).await?;
        self.record(actor, node, ActivityKind::Created).await
    }
}

/// The root `.vellum-folder.yaml` of a space.
pub(super) fn space_file(space: &Space) -> FolderFile {
    FolderFile {
        id: space.node_id(),
        icon: None,
        space: Some(SpaceFile {
            kind: space.kind,
            name: space.name.clone(),
            owner: space.owner,
            settings: space.settings.clone(),
        }),
    }
}
