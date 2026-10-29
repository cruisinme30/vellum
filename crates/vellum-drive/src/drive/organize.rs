// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Renaming, moving and copying, with link rewriting.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use vellum_kernel::{Actor, ObjectId, SpaceId};
use vellum_store::{BlobPath, Precondition, StoreError};

use super::Drive;
use crate::error::{DriveError, DriveResult};
use crate::files::{ShortcutFile, from_yaml, to_yaml};
use crate::frontmatter;
use crate::layout;
use crate::links::{self, Target};
use crate::model::{ActivityKind, Node, NodeKind, Space, SpaceKind};
use crate::ports::Permission;

/// Nodes that moved: (before, after).
type Moved = Vec<(Node, Node)>;

impl Drive {
    /// Renames a node. Pages get a new front-matter title and file name;
    /// folders, books and collections a new directory name. Relative links
    /// and wikilinks pointing at renamed pages are rewritten; ID links need
    /// nothing (R-DR-5).
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`, `Invalid` for an empty title.
    #[tracing::instrument(name = "drive.rename", skip_all, fields(node = %id), err)]
    pub async fn rename(&self, actor: &Actor, id: ObjectId, title: &str) -> DriveResult<Node> {
        self.observe("rename", async {
            let node = self.live_node(id).await?;
            self.authorize(actor, node.space, node.id, Permission::Edit)
                .await?;
            let title = title.trim();
            if title.is_empty() {
                return Err(DriveError::Invalid("a title cannot be empty".into()));
            }
            if title == node.title {
                return Ok(node);
            }
            match node.kind {
                NodeKind::Page => {
                    // Saving a new front-matter title renames through retitle.
                    let title = title.to_owned();
                    self.rewrite_page(actor, node, move |text| {
                        Some(frontmatter::set_md(text, "title", &title))
                    })
                    .await
                }
                NodeKind::Shortcut => {
                    let (_, blob) = self.read_text(&node.path).await?;
                    let mut file: ShortcutFile = from_yaml(&node.path, &blob.bytes)?;
                    title.clone_into(&mut file.title);
                    let _ = self.put_file(&node.path, to_yaml(&file)?).await?;
                    self.retitle(actor, node, title).await
                }
                NodeKind::Book | NodeKind::Collection => {
                    let file = layout::id_file(node.kind, &node.path)?;
                    let (text, _) = self.read_text(&file).await?;
                    let _ = self
                        .put_file(&file, frontmatter::set(&text, "title", title))
                        .await?;
                    self.retitle(actor, node, title).await
                }
                NodeKind::Folder => self.retitle(actor, node, title).await,
            }
        })
        .await
    }

    /// Gives a node a new title and file name in its current folder and
    /// fixes links. The file content already carries the new title.
    pub(super) async fn retitle(
        &self,
        actor: &Actor,
        node: Node,
        title: &str,
    ) -> DriveResult<Node> {
        let dir = node
            .path
            .parent()
            .ok_or_else(|| DriveError::Invalid("a node cannot live at the store root".into()))?;
        let path = self
            .free_path(&dir, node.kind, title, Some(&node.path))
            .await?;
        let space = self.space(node.space).await?;
        let old_title = node.title.clone();
        let parent = node.parent;
        let moved = self
            .relocate(&node, &space, parent, path, Some(title))
            .await?;
        let renamed = moved
            .first()
            .map(|(_, after)| after.clone())
            .ok_or(DriveError::NotFound(node.id))?;
        self.fix_links(actor, &moved).await?;
        self.record(
            actor,
            &renamed,
            ActivityKind::Renamed {
                from: old_title,
                to: title.to_owned(),
            },
        )
        .await?;
        Ok(renamed)
    }

    /// Moves a node (and everything beneath a folder) to `parent` in
    /// `space`. A moved node keeps the grants made on it and drops the ones
    /// it inherited (R-DR-2). Leaving a shared drive needs manager rights.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`, `Invalid` (into itself, not a
    /// folder), `QuotaExceeded` in the destination.
    #[tracing::instrument(name = "drive.move", skip_all, fields(node = %id), err)]
    pub async fn move_node(
        &self,
        actor: &Actor,
        id: ObjectId,
        space: SpaceId,
        parent: Option<ObjectId>,
    ) -> DriveResult<Node> {
        self.observe("move", async {
            let node = self.live_node(id).await?;
            let from_space = self.space(node.space).await?;
            let to_space = self.space(space).await?;
            let permission =
                if from_space.id != to_space.id && from_space.kind == SpaceKind::SharedDrive {
                    Permission::Manage
                } else {
                    Permission::Edit
                };
            self.authorize(actor, node.space, node.id, permission)
                .await?;
            self.authorize(
                actor,
                to_space.id,
                parent.unwrap_or_else(|| to_space.node_id()),
                Permission::Edit,
            )
            .await?;
            let dir = self.dir_of(&to_space, parent).await?;
            if dir.starts_with(&node.path) {
                return Err(DriveError::Invalid(
                    "cannot move a folder into itself".into(),
                ));
            }
            if node.path.parent().as_ref() == Some(&dir) {
                return Ok(node);
            }
            if from_space.id != to_space.id {
                let bytes = self.subtree_bytes(&node).await?;
                self.ensure_quota(&to_space, bytes).await?;
            }
            let path = self.free_path(&dir, node.kind, &node.title, None).await?;
            let from = node.path.to_string();
            let moved = self.relocate(&node, &to_space, parent, path, None).await?;
            let after = moved
                .first()
                .map(|(_, after)| after.clone())
                .ok_or(DriveError::NotFound(node.id))?;
            self.fix_links(actor, &moved).await?;
            let ids: Vec<ObjectId> = moved.iter().map(|(_, n)| n.id).collect();
            self.access.tree_changed(from_space.id, &ids).await?;
            if from_space.id != to_space.id {
                self.access.tree_changed(to_space.id, &ids).await?;
            }
            self.record(
                actor,
                &after,
                ActivityKind::Moved {
                    from,
                    to: after.path.to_string(),
                },
            )
            .await?;
            Ok(after)
        })
        .await
    }

    /// Copies a node (a folder with everything in it) into `parent` of
    /// `space`. Every copied node gets a fresh ID, and links inside the
    /// copy are re-pointed at the copies (R-DR-3). The copier owns the
    /// copy; no grants are copied.
    ///
    /// # Errors
    /// `NotFound`, `PermissionDenied`, `Invalid`, `QuotaExceeded`.
    #[tracing::instrument(name = "drive.copy", skip_all, fields(node = %id), err)]
    pub async fn copy_node(
        &self,
        actor: &Actor,
        id: ObjectId,
        space: SpaceId,
        parent: Option<ObjectId>,
    ) -> DriveResult<Node> {
        self.observe("copy", async {
            let node = self.live_node(id).await?;
            self.authorize(actor, node.space, node.id, Permission::View)
                .await?;
            let to_space = self.space(space).await?;
            self.authorize(
                actor,
                to_space.id,
                parent.unwrap_or_else(|| to_space.node_id()),
                Permission::Edit,
            )
            .await?;
            let dir = self.dir_of(&to_space, parent).await?;
            if dir.starts_with(&node.path) {
                return Err(DriveError::Invalid(
                    "cannot copy a folder into itself".into(),
                ));
            }
            let blobs = if node.kind.is_dir() {
                // Build output and in-flight temp files are not content.
                self.blobs_under(&node.path)
                    .await?
                    .into_iter()
                    .filter(|(p, _)| {
                        !p.segments().any(|s| s == ".mdbook")
                            && !p.file_name().contains(".vellum-tmp")
                            && vellum_store::layout::Kind::of(p).is_content()
                    })
                    .collect()
            } else {
                vec![(node.path.clone(), node.size)]
            };
            let total: u64 = blobs.iter().map(|(_, size)| size).sum();
            self.ensure_quota(&to_space, total).await?;
            let dest = self.free_path(&dir, node.kind, &node.title, None).await?;

            // Read everything first, then map every ID found to a fresh one.
            let mut files = Vec::with_capacity(blobs.len());
            let mut ids = HashMap::new();
            for (path, _) in blobs {
                let blob = self.store.get(&path).await?;
                if let Some(text) = text_of(&path, &blob.bytes) {
                    let id = if layout::has_ext(path.file_name(), "md") {
                        frontmatter::get_md(&text, "id")
                    } else {
                        frontmatter::get(&text, "id")
                    };
                    if let Some(id) = id.filter(|id| id.parse::<ObjectId>().is_ok()) {
                        let _ = ids
                            .entry(id)
                            .or_insert_with(|| ObjectId::generate().to_string());
                    }
                }
                files.push((path, blob.bytes));
            }
            for (path, bytes) in files {
                let target = layout::rebase(&path, &node.path, &dest)?;
                let bytes = text_of(&path, &bytes)
                    .map_or(bytes, |text| Bytes::from(links::replace_ids(&text, &ids)));
                let _ = self
                    .store
                    .put(&target, bytes, Precondition::IfAbsent)
                    .await?;
            }
            let copies = self
                .index_tree(
                    &to_space,
                    &dest,
                    node.kind.is_dir(),
                    parent,
                    Self::user_of(actor),
                )
                .await?;
            let top = copies
                .iter()
                .find(|n| n.path == dest)
                .cloned()
                .ok_or_else(|| DriveError::Invalid("the copy could not be indexed".into()))?;
            if let Some(user) = Self::user_of(actor) {
                self.access.grant_owner(user, top.space, top.id).await?;
            }
            let ids: Vec<ObjectId> = copies.iter().map(|n| n.id).collect();
            self.access.tree_changed(to_space.id, &ids).await?;
            self.record(actor, &top, ActivityKind::Copied { from: node.id })
                .await?;
            Ok(top)
        })
        .await
    }

    /// Bytes a node and its subtree use, versions included.
    pub(super) async fn subtree_bytes(&self, node: &Node) -> DriveResult<u64> {
        let mut total = 0_u64;
        for member in self.subtree(node).await? {
            total = total.saturating_add(member.size);
            for version in self.index.versions(member.id).await? {
                total = total.saturating_add(version.size);
            }
        }
        Ok(total)
    }

    /// The node and, for a folder, every live node beneath it.
    pub(super) async fn subtree(&self, node: &Node) -> DriveResult<Vec<Node>> {
        let mut out = vec![node.clone()];
        if node.kind.is_container() {
            out.extend(
                self.index
                    .nodes(node.space)
                    .await?
                    .into_iter()
                    .filter(|n| n.is_live() && n.id != node.id && n.path.starts_with(&node.path)),
            );
        }
        Ok(out)
    }

    /// Moves a node's files to `path` in `space` under `parent`, updates
    /// the index for it and everything beneath it, and returns each moved
    /// node before and after.
    pub(super) async fn relocate(
        &self,
        node: &Node,
        space: &Space,
        parent: Option<ObjectId>,
        path: BlobPath,
        title: Option<&str>,
    ) -> DriveResult<Moved> {
        let members = self.subtree(node).await?;
        self.move_blobs(node.kind, &node.path, &path).await?;
        let now = self.clock.now();
        let mut moved = Vec::with_capacity(members.len());
        for before in members {
            let mut after = before.clone();
            after.path = layout::rebase(&before.path, &node.path, &path)?;
            after.space = space.id;
            if before.id == node.id {
                after.parent = parent;
                after.modified = now;
                after.title = match (title, node.kind) {
                    (Some(title), NodeKind::Page | NodeKind::Shortcut) => title.to_owned(),
                    (Some(title), _) => {
                        let name = layout::title_from_path(node.kind, &path);
                        if name == layout::stem(title) {
                            title.to_owned()
                        } else {
                            name
                        }
                    }
                    (None, _) => before.title.clone(),
                };
            }
            if before.space != space.id {
                for mut version in self.index.versions(before.id).await? {
                    version.space = space.id;
                    self.index.put_version(&version).await?;
                }
            }
            self.index.put_node(&after).await?;
            moved.push((before, after));
        }
        Ok(moved)
    }

    /// Rewrites relative links and wikilinks broken by `moved`:
    /// links inside moved pages that point elsewhere, and links in other
    /// pages that point at moved or renamed pages.
    pub(super) async fn fix_links(&self, actor: &Actor, moved: &Moved) -> DriveResult<()> {
        let paths: HashMap<BlobPath, BlobPath> = moved
            .iter()
            .map(|(before, after)| (before.path.clone(), after.path.clone()))
            .collect();
        let mut titles: HashMap<String, String> = HashMap::new();
        for (before, after) in moved {
            if after.kind == NodeKind::Page && before.title != after.title {
                let still_taken = !self
                    .index
                    .by_title(before.space, &before.title)
                    .await?
                    .is_empty();
                if !still_taken {
                    let _ = titles.insert(before.title.clone(), after.title.clone());
                }
            }
        }
        let moved_ids: HashSet<ObjectId> = moved.iter().map(|(_, n)| n.id).collect();

        // Links inside moved pages, and inside moved books and collections.
        for (before, after) in moved {
            if before.path.parent() == after.path.parent() && before.kind != NodeKind::Folder {
                continue;
            }
            match after.kind {
                NodeKind::Page => {
                    let (old_dir, new_dir) = (before.path.parent(), after.path.parent());
                    let paths = paths.clone();
                    let _ = self
                        .relink(actor, after.clone(), move |text| {
                            relink_text(
                                text,
                                old_dir.as_ref(),
                                new_dir.as_ref(),
                                &paths,
                                &HashMap::new(),
                            )
                        })
                        .await?;
                }
                NodeKind::Book | NodeKind::Collection => {
                    self.relink_inner(&before.path, &after.path, &paths).await?;
                }
                NodeKind::Folder | NodeKind::Shortcut => {}
            }
        }

        // Links pointing at moved or renamed pages from elsewhere.
        let mut sources = HashSet::new();
        for (_, after) in moved {
            if after.kind == NodeKind::Page {
                sources.extend(self.index.backlinks(after.id).await?);
            }
        }
        for source in sources {
            if moved_ids.contains(&source) {
                continue;
            }
            let Some(node) = self.index.node(source).await? else {
                continue;
            };
            if !node.is_live() || node.kind != NodeKind::Page {
                continue;
            }
            let dir = node.path.parent();
            let (paths, titles) = (paths.clone(), titles.clone());
            let _ = self
                .relink(actor, node, move |text| {
                    relink_text(text, dir.as_ref(), dir.as_ref(), &paths, &titles)
                })
                .await?;
        }
        Ok(())
    }

    /// [`Self::rewrite_page`], boxed to break the save → rename → relink
    /// → save cycle into a finite future.
    fn relink<'a>(
        &'a self,
        actor: &'a Actor,
        node: Node,
        edit: impl FnOnce(&str) -> Option<String> + Send + 'a,
    ) -> Pin<Box<dyn Future<Output = DriveResult<Node>> + Send + 'a>> {
        Box::pin(self.rewrite_page(actor, node, edit))
    }

    /// Fixes outward relative links in the Markdown files of a moved book
    /// or collection, whose inner pages the drive does not index.
    async fn relink_inner(
        &self,
        from: &BlobPath,
        to: &BlobPath,
        paths: &HashMap<BlobPath, BlobPath>,
    ) -> DriveResult<()> {
        for (path, _) in self.blobs_under(to).await? {
            if !layout::has_ext(path.file_name(), "md") {
                continue;
            }
            let old = layout::rebase(&path, to, from)?;
            let (text, blob) = self.read_text(&path).await?;
            let mut map = paths.clone();
            // Files inside the moved directory moved with it.
            for (inner, _) in self.blobs_under(to).await? {
                let _ = map.insert(layout::rebase(&inner, to, from)?, inner);
            }
            if let Some(new) = relink_text(
                &text,
                old.parent().as_ref(),
                path.parent().as_ref(),
                &map,
                &HashMap::new(),
            ) {
                match self
                    .store
                    .put(
                        &path,
                        Bytes::from(new),
                        Precondition::IfMatch(blob.meta.etag),
                    )
                    .await
                {
                    // Someone edited it meanwhile; their version wins and the
                    // next scan re-reads it.
                    Ok(_) | Err(StoreError::PreconditionFailed { .. }) => {}
                    Err(err) => return Err(err.into()),
                }
            }
        }
        Ok(())
    }
}

/// Rewrites the relative links and wikilinks of a page that lived in
/// `old_dir` and now lives in `new_dir`. Returns `None` when nothing
/// changes.
fn relink_text(
    text: &str,
    old_dir: Option<&BlobPath>,
    new_dir: Option<&BlobPath>,
    paths: &HashMap<BlobPath, BlobPath>,
    titles: &HashMap<String, String>,
) -> Option<String> {
    let mut edits = Vec::new();
    for link in links::scan(text) {
        match link.target {
            Target::Relative(rel) => {
                let Some(target) = links::resolve(old_dir, &rel) else {
                    continue;
                };
                let target = paths.get(&target).cloned().unwrap_or(target);
                let new = links::relative(new_dir, &target);
                if links::resolve(new_dir, &rel).as_ref() != Some(&target) {
                    edits.push((link.span, links::percent_encode(&new)));
                }
            }
            Target::Wiki(title) => {
                if let Some(new) = titles.get(&title) {
                    edits.push((link.span, new.clone()));
                }
            }
            Target::Id(_) => {}
        }
    }
    (!edits.is_empty()).then(|| links::apply(text, edits))
}

/// The text of a file the drive may rewrite IDs in.
fn text_of(path: &BlobPath, bytes: &[u8]) -> Option<String> {
    let name = path.file_name();
    let texty = ["md", "yaml", "yml", "json"]
        .iter()
        .any(|ext| layout::has_ext(name, ext));
    if !texty {
        return None;
    }
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> BlobPath {
        BlobPath::new(s).unwrap_or_else(|e| unreachable!("{e}"))
    }

    #[test]
    fn relinks_moved_pages_and_their_targets() {
        let paths = HashMap::from([(p("d/a/Plan.md"), p("d/b/c/Plan.md"))]);
        // A page that moved from d/a to d/b/c, linking to a sibling that stayed.
        let out = relink_text(
            "[s](Other.md) [p](Plan.md)",
            Some(&p("d/a")),
            Some(&p("d/b/c")),
            &paths,
            &HashMap::new(),
        );
        assert_eq!(out.as_deref(), Some("[s](../../a/Other.md) [p](Plan.md)"));
        // A page that stayed in d, linking to the moved page.
        let out = relink_text(
            "[p](a/Plan.md) [[Plan]]",
            Some(&p("d")),
            Some(&p("d")),
            &paths,
            &HashMap::from([("Plan".to_owned(), "Plan v2".to_owned())]),
        );
        assert_eq!(out.as_deref(), Some("[p](b/c/Plan.md) [[Plan v2]]"));
        // Nothing to change.
        assert_eq!(
            relink_text(
                "[x](y.md)",
                Some(&p("d")),
                Some(&p("d")),
                &paths,
                &HashMap::new()
            ),
            None
        );
    }
}
