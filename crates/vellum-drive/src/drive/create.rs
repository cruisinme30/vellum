// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Creating spaces, folders, pages and shortcuts, and indexing the books
//! and collections other services write.

use bytes::Bytes;
use futures::TryStreamExt;
use vellum_kernel::{Actor, ObjectId, SpaceId, UserId};
use vellum_store::{BlobPath, Precondition};

use super::Drive;
use crate::error::{DriveError, DriveResult};
use crate::files::{FolderFile, ShortcutFile, SpaceFile, to_yaml};
use crate::frontmatter;
use crate::layout;
use crate::model::{
    ActivityKind, Node, NodeKind, Space, SpaceKind, SpaceSettings,
};
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
    #[tracing::instrument(
        name = "drive.create_space",
        skip_all,
        fields(slug = slug),
        err
    )]
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
                    return Err(DriveError::Invalid(
                        "a My Drive needs an owner".into(),
                    ));
                }
                (SpaceKind::UserDrive, Some(owner), Actor::User { user })
                    if owner != *user =>
                {
                    return Err(DriveError::PermissionDenied);
                }
                (_, _, Actor::Model { .. }) => {
                    return Err(DriveError::PermissionDenied);
                }
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

    /// The person's My Drive, created when they have none yet (D171).
    /// Vellum creates it as itself, owned by `user`, under the slug
    /// [`layout::slug_of`] makes from `handle`, or under that slug plus
    /// the end of the user's ID when someone else holds it. Safe to call
    /// at once from several places: a drive another caller just wrote for
    /// the same person is found and kept.
    ///
    /// # Errors
    /// Store or index failures.
    #[tracing::instrument(name = "drive.ensure_my_drive", skip_all, err)]
    pub async fn ensure_my_drive(
        &self,
        user: UserId,
        handle: Option<&str>,
    ) -> DriveResult<Space> {
        if let Some(space) = self.my_drive(user).await? {
            return Ok(space);
        }
        let base = handle.map_or_else(|| "me".to_owned(), layout::slug_of);
        let id = user.to_string().to_lowercase();
        let tail = id.get(id.len().saturating_sub(8)..).unwrap_or(&id);
        let mut suffixed = base.clone();
        suffixed.truncate(48);
        let candidates =
            [base.clone(), format!("{suffixed}-{tail}"), id.clone()];
        for slug in &candidates {
            match self
                .create_space(
                    &Actor::System,
                    SpaceKind::UserDrive,
                    slug,
                    MY_DRIVE_NAME,
                    Some(user),
                )
                .await
            {
                Ok(space) => return Ok(space),
                Err(DriveError::AlreadyExists(_)) => {
                    if let Some(space) = self.claimed_by(user, slug).await? {
                        return Ok(space);
                    }
                }
                Err(err) => return Err(err),
            }
        }
        Err(DriveError::AlreadyExists(base))
    }

    /// The My Drive at `slug` if its folder file says it belongs to
    /// `user`, indexed as it stands.
    async fn claimed_by(
        &self,
        user: UserId,
        slug: &str,
    ) -> DriveResult<Option<Space>> {
        let root = layout::space_root(SpaceKind::UserDrive, slug)?;
        let path = root.join(layout::FOLDER_FILE)?;
        let blob = match self.store.get(&path).await {
            Ok(blob) => blob,
            Err(vellum_store::StoreError::NotFound(_)) => return Ok(None),
            Err(err) => return Err(err.into()),
        };
        let file: FolderFile = crate::files::from_yaml(&path, &blob.bytes)?;
        let Some(meta) = file.space.filter(|m| m.owner == Some(user)) else {
            return Ok(None);
        };
        let space = Space {
            id: file.id.into(),
            kind: SpaceKind::UserDrive,
            slug: slug.to_owned(),
            name: meta.name,
            owner: meta.owner,
            settings: meta.settings,
            root,
        };
        self.index.put_space(&space).await?;
        Ok(Some(space))
    }

    /// Changes a space's display name and settings (quota, trash days,
    /// version retention). Managers only.
    ///
    /// # Errors
    /// `SpaceNotFound`, `PermissionDenied`, or a store failure.
    #[tracing::instrument(
        name = "drive.update_space",
        skip_all,
        fields(space = %space),
        err
    )]
    pub async fn update_space(
        &self,
        actor: &Actor,
        space: SpaceId,
        name: Option<&str>,
        settings: SpaceSettings,
    ) -> DriveResult<Space> {
        self.observe("update_space", async {
            let mut space = self.space(space).await?;
            self.authorize(
                actor,
                space.id,
                space.node_id(),
                Permission::Manage,
            )
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
    #[tracing::instrument(
        name = "drive.create_folder",
        skip_all,
        fields(space = %space),
        err
    )]
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
            let path =
                self.free_path(&dir, NodeKind::Folder, name, None).await?;
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
    #[tracing::instrument(
        name = "drive.create_page",
        skip_all,
        fields(space = %space),
        err
    )]
    pub async fn create_page(
        &self,
        actor: &Actor,
        space: SpaceId,
        parent: Option<ObjectId>,
        title: &str,
        body: &str,
    ) -> DriveResult<Node> {
        self.create_page_with(actor, space, parent, title, body, &[])
            .await
    }

    /// As [`Drive::create_page`], with extra front-matter fields written
    /// after `id` and `title`, such as `status: draft` and the attribution
    /// of a page a model wrote (ADR 0019, D218).
    ///
    /// # Errors
    /// `PermissionDenied`, `Invalid`, or `QuotaExceeded`.
    pub async fn create_page_with(
        &self,
        actor: &Actor,
        space: SpaceId,
        parent: Option<ObjectId>,
        title: &str,
        body: &str,
        front: &[(&str, &str)],
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
            let path =
                self.free_path(&dir, NodeKind::Page, title, None).await?;
            let id = ObjectId::generate();
            let mut yaml = frontmatter::set("", "id", &id.to_string());
            yaml = frontmatter::set(&yaml, "title", title);
            for (key, value) in front {
                if matches!(*key, "id" | "title" | "vellum") {
                    return Err(DriveError::Invalid(format!(
                        "{key} is set by the drive"
                    )));
                }
                yaml = frontmatter::set(&yaml, key, value);
            }
            let text = format!("---\n{yaml}vellum: 1\n---\n\n{body}");
            self.check_content(actor, space.id, &path, text.as_bytes())
                .await?;
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
    #[tracing::instrument(
        name = "drive.create_shortcut",
        skip_all,
        fields(target = %target),
        err
    )]
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

    /// Creates an empty book titled `title` in the drive folder `folder`
    /// (a space's root folder or a live folder): `Title.book/` with its
    /// `book.yaml` and `SUMMARY.md`, owned by its creator (D322).
    ///
    /// # Errors
    /// `Invalid` for a blank title or a path that is not such a folder,
    /// `PermissionDenied` without edit rights on it, `QuotaExceeded`.
    #[tracing::instrument(name = "drive.create_book", skip_all, fields(folder = %folder), err)]
    pub async fn create_book(
        &self,
        actor: &Actor,
        folder: &BlobPath,
        title: &str,
    ) -> DriveResult<Node> {
        self.observe("create_book", async {
            let title = title.trim();
            if title.is_empty() {
                return Err(DriveError::Invalid("a book needs a title".into()));
            }
            let (space, parent) = self.editable_folder(actor, folder).await?;
            let path =
                self.free_path(folder, NodeKind::Book, title, None).await?;
            let id = ObjectId::generate();
            let book_id = vellum_books::BookId::parse(&id.to_string())
                .map_err(|e| {
                    DriveError::Invalid(format!("bad book id: {e}"))
                })?;
            let book = vellum_books::Book::new(book_id, title);
            let manifest = book.manifest_text().map_err(|e| {
                DriveError::Invalid(format!("cannot write book.yaml: {e}"))
            })?;
            let summary = book.summary_text();
            let size = (manifest.len() + summary.len()) as u64;
            self.ensure_quota(&space, size).await?;
            let _ = self
                .store
                .put(
                    &layout::id_file(NodeKind::Book, &path)?,
                    Bytes::from(manifest),
                    Precondition::IfAbsent,
                )
                .await?;
            let _ = self
                .store
                .put(
                    &path.join(layout::BOOK_SUMMARY_FILE)?,
                    Bytes::from(summary),
                    Precondition::IfAbsent,
                )
                .await?;
            let node = Node {
                id,
                space: space.id,
                kind: NodeKind::Book,
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
            Ok(node)
        })
        .await
    }

    /// The space and parent node of `dir`, a space's root folder or a live
    /// folder, after checking `actor` may edit there (D305).
    async fn editable_folder(
        &self,
        actor: &Actor,
        dir: &BlobPath,
    ) -> DriveResult<(Space, Option<ObjectId>)> {
        let space = self
            .index
            .spaces()
            .await?
            .into_iter()
            .find(|s| dir.starts_with(&s.root))
            .ok_or_else(|| {
                DriveError::Invalid(format!("{dir} is not in a space"))
            })?;
        let parent = if *dir == space.root {
            None
        } else {
            match self.index.node_at(space.id, dir).await? {
                Some(n) if n.kind == NodeKind::Folder && n.is_live() => {
                    Some(n.id)
                }
                _ => {
                    return Err(DriveError::Invalid(format!(
                        "{dir} is not a folder"
                    )));
                }
            }
        };
        self.authorize(
            actor,
            space.id,
            parent.unwrap_or_else(|| space.node_id()),
            Permission::Edit,
        )
        .await?;
        Ok((space, parent))
    }

    /// Indexes a book or collection another service has just written at
    /// `path`, as if the drive had made it: the creator owns it, and it
    /// is in the folder tree (and so in permissions) at once (D321).
    /// Its manifest (`book.yaml`, `schema.yaml`) carries its ID and title.
    /// Only a space's root folder or a live folder can hold it, as for
    /// creating a collection (D305). Indexing it again is a no-op.
    ///
    /// # Errors
    /// `Invalid` for any other path or a manifest without an ID,
    /// `PermissionDenied` without edit rights on the folder,
    /// `AlreadyExists` if another node holds the ID.
    #[tracing::instrument(name = "drive.add_unit", skip_all, fields(path = %path), err)]
    pub async fn add_unit(
        &self,
        actor: &Actor,
        path: &BlobPath,
    ) -> DriveResult<Node> {
        self.observe("add_unit", async {
            let name = path.file_name();
            let kind = [NodeKind::Book, NodeKind::Collection]
                .into_iter()
                .find(|k| {
                    name.len() > k.suffix().len() && name.ends_with(k.suffix())
                })
                .ok_or_else(|| {
                    DriveError::Invalid(format!(
                        "{path} is not a book or collection"
                    ))
                })?;
            let dir = path.parent().ok_or_else(|| {
                DriveError::Invalid(format!("{path} is not in a space"))
            })?;
            let (space, parent) = self.editable_folder(actor, &dir).await?;
            let (text, _) =
                self.read_text(&layout::id_file(kind, path)?).await?;
            let id: ObjectId = frontmatter::get(&text, "id")
                .and_then(|id| id.parse().ok())
                .ok_or_else(|| {
                    DriveError::Invalid(format!("{path} has no id"))
                })?;
            if let Some(known) = self.index.node(id).await? {
                if known.path == *path && known.is_live() {
                    return Ok(known);
                }
                return Err(DriveError::AlreadyExists(id.to_string()));
            }
            let mut size = 0_u64;
            let mut files = self.store.list(Some(path));
            while let Some(entry) = files.try_next().await? {
                size = size.saturating_add(entry.size);
            }
            let node = Node {
                id,
                space: space.id,
                kind,
                parent,
                title: frontmatter::get(&text, "title")
                    .map(|t| t.trim().to_owned())
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| layout::title_from_path(kind, path)),
                path: path.clone(),
                size,
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

    /// Indexes a new node, makes its creator its owner and records it.
    pub(super) async fn created(
        &self,
        actor: &Actor,
        node: &Node,
    ) -> DriveResult<()> {
        self.index.put_node(node).await?;
        if let Some(user) = Self::user_of(actor) {
            self.access.grant_owner(user, node.space, node.id).await?;
            self.index.touch(user, node.id, node.modified).await?;
        }
        self.access.tree_changed(node.space, &[node.id]).await?;
        self.record(actor, node, ActivityKind::Created).await
    }
}

/// The name a My Drive gets when Vellum creates it.
pub const MY_DRIVE_NAME: &str = "My Drive";

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
