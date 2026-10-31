// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Rebuilding the index from the files (`vellum reindex`) and
//! reconciling changes made outside Vellum.
//!
//! Identity is the ID in each file, never the path (D11): a file found at a
//! new path with a known ID was renamed or moved outside the app, and its
//! node simply follows it. Two files with the same ID (a copy made in
//! Finder) are told apart by giving the newcomer a fresh ID.

use std::collections::{BTreeMap, HashMap, HashSet};

use bytes::Bytes;
use futures::TryStreamExt;
use vellum_kernel::{ObjectId, SpaceId, UserId, VersionId};
use vellum_store::{BlobPath, Precondition};

use super::Drive;
use crate::error::DriveResult;
use crate::files::{FolderFile, ShortcutFile, TrashManifest, from_yaml};
use crate::frontmatter;
use crate::layout;
use crate::model::{
    Node, NodeKind, ScanReport, Space, TrashState, VersionRecord,
};

/// Blob paths and sizes, sorted by path.
type Blobs = BTreeMap<BlobPath, u64>;

/// The IDs a scan has handed out. With `fresh` (a copy or an import,
/// whose files carry IDs someone else chose), an ID the index already
/// holds anywhere counts as taken too (RT-037).
struct Taken<'a> {
    ids: &'a mut HashSet<ObjectId>,
    fresh: bool,
}

impl Drive {
    /// Rebuilds the drive's part of the index from the store: spaces,
    /// nodes, trash, versions and links. Per-user state and activity, which
    /// files do not carry, are kept.
    ///
    /// # Errors
    /// Store or index failures. Unreadable files are skipped and counted.
    #[tracing::instrument(name = "drive.scan", skip_all, err)]
    pub async fn scan(&self) -> DriveResult<ScanReport> {
        self.observe("scan", async {
            let entries: Vec<_> = self.store.list(None).try_collect().await?;
            let blobs: Blobs =
                entries.into_iter().map(|e| (e.path, e.size)).collect();
            let mut report = ScanReport::default();
            let spaces = self.scan_spaces(&blobs, &mut report).await?;
            let manifests = self.read_manifests(&blobs, &mut report).await;
            let mut seen_ids = HashSet::new();
            for space in &spaces {
                self.scan_space(
                    space,
                    &blobs,
                    &manifests,
                    &mut seen_ids,
                    &mut report,
                )
                .await?;
                // Nodes found on disk inherit access like any other, so
                // the permission engine needs the space's tree as scanned.
                let ids: Vec<ObjectId> = self
                    .index
                    .nodes(space.id)
                    .await?
                    .iter()
                    .map(|n| n.id)
                    .collect();
                self.access.tree_changed(space.id, &ids).await?;
            }
            self.scan_versions(&blobs).await?;
            Ok(report)
        })
        .await
    }

    /// Indexes the nodes found at and beneath `dir` in `space` (a copy or
    /// an import just written), returning them. Every ID the index already
    /// holds is re-identified, so new files never take over existing
    /// nodes (RT-037).
    pub(super) async fn index_tree(
        &self,
        space: &Space,
        dir: &BlobPath,
        is_dir: bool,
        parent: Option<ObjectId>,
        creator: Option<UserId>,
    ) -> DriveResult<Vec<Node>> {
        let mut blobs = Blobs::new();
        if is_dir {
            blobs.extend(self.blobs_under(dir).await?);
        } else {
            let meta = self.store.head(dir).await?;
            let _ = blobs.insert(meta.path, meta.size);
            // A copied page brings the pages nested under it (D168).
            if let Some(nested) = self.children_dir(NodeKind::Page, dir).await?
            {
                blobs.extend(self.blobs_under(&nested).await?);
            }
        }
        let mut seen: HashSet<ObjectId> = HashSet::new();
        let mut report = ScanReport::default();
        let mut found = self
            .discover(
                space,
                &blobs,
                &HashSet::new(),
                &mut seen,
                true,
                &mut report,
            )
            .await?;
        let mut holders = self.container_paths(space.id).await?;
        holders.extend(containers(found.iter().map(|(n, _)| n)));
        let mut out = Vec::with_capacity(found.len());
        for (node, text) in &mut found {
            node.parent = if &node.path == dir {
                parent
            } else {
                parent_of(&node.path, &holders)
            };
            node.created_by = creator;
            self.index.put_node(node).await?;
            out.push((node.clone(), text.take()));
        }
        for (node, text) in &out {
            if let Some(text) = text {
                self.reindex_links(node, text).await?;
            }
        }
        Ok(out.into_iter().map(|(node, _)| node).collect())
    }

    async fn scan_spaces(
        &self,
        blobs: &Blobs,
        report: &mut ScanReport,
    ) -> DriveResult<Vec<Space>> {
        let mut spaces = Vec::new();
        for path in blobs.keys() {
            if path.file_name() != layout::FOLDER_FILE {
                continue;
            }
            let Some(root) = path.parent() else { continue };
            let Some((kind, slug)) = layout::parse_space_root(&root) else {
                continue;
            };
            let file: FolderFile = match self.read_yaml(path).await {
                Ok(file) => file,
                Err(err) => {
                    tracing::warn!(%path, error = %err, "skipping unreadable \
                        space");
                    report.skipped += 1;
                    continue;
                }
            };
            let Some(meta) = file.space else { continue };
            let space = Space {
                id: file.id.into(),
                kind,
                slug: slug.to_owned(),
                name: meta.name,
                owner: meta.owner,
                settings: meta.settings,
                root,
            };
            self.index.put_space(&space).await?;
            spaces.push(space);
        }
        report.spaces = spaces.len();
        Ok(spaces)
    }

    pub(super) async fn read_manifests(
        &self,
        blobs: &Blobs,
        report: &mut ScanReport,
    ) -> Vec<(ObjectId, TrashManifest)> {
        let mut out = Vec::new();
        for path in blobs.keys() {
            let mut parts = path.segments();
            let (
                Some(layout::STATE_DIR),
                Some("trash"),
                Some(entry),
                Some(layout::TRASH_MANIFEST),
                None,
            ) = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            )
            else {
                continue;
            };
            let Ok(entry) = entry.parse::<ObjectId>() else {
                continue;
            };
            match self.read_yaml::<TrashManifest>(path).await {
                Ok(manifest) => out.push((entry, manifest)),
                Err(err) => {
                    tracing::warn!(%path, error = %err, "skipping unreadable \
                        trash manifest");
                    report.skipped += 1;
                }
            }
        }
        out
    }

    pub(super) async fn scan_space(
        &self,
        space: &Space,
        all: &Blobs,
        manifests: &[(ObjectId, TrashManifest)],
        seen: &mut HashSet<ObjectId>,
        report: &mut ScanReport,
    ) -> DriveResult<()> {
        let existing: HashMap<ObjectId, Node> = self
            .index
            .nodes(space.id)
            .await?
            .into_iter()
            .map(|n| (n.id, n))
            .collect();
        let blobs: Blobs = all
            .range(space.root.clone()..)
            .take_while(|(p, _)| p.starts_with(&space.root))
            .map(|(p, s)| (p.clone(), *s))
            .collect();
        let mut known = HashSet::new();
        for node in existing.values().filter(|n| n.is_live()) {
            let _ = known.insert(layout::id_file(node.kind, &node.path)?);
        }
        let mut found = self
            .discover(space, &blobs, &known, seen, false, report)
            .await?;
        let holders = containers(found.iter().map(|(n, _)| n));
        for (node, _) in &mut found {
            node.parent = parent_of(&node.path, &holders);
            if let Some(old) = existing.get(&node.id) {
                node.created_by = old.created_by;
                if old.path != node.path || !old.is_live() {
                    report.relocated += 1;
                } else {
                    node.modified = old.modified;
                }
            } else {
                report.added += 1;
            }
            self.index.put_node(node).await?;
        }
        let mut live: HashSet<ObjectId> =
            found.iter().map(|(n, _)| n.id).collect();

        for (entry, manifest) in
            manifests.iter().filter(|(_, m)| m.space == space.id)
        {
            let Some(top) = manifest.members.first() else {
                continue;
            };
            let item =
                layout::trash_item(*entry, top.original_path.file_name())?;
            for member in &manifest.members {
                if live.contains(&member.id) {
                    continue;
                }
                let path = layout::rebase_member(
                    top.kind,
                    &member.original_path,
                    &top.original_path,
                    &item,
                )?;
                let size = if member.kind.is_dir() {
                    all.range(path.clone()..)
                        .take_while(|(p, _)| p.starts_with(&path))
                        .map(|(_, s)| *s)
                        .sum()
                } else {
                    all.get(&path).copied().unwrap_or(0)
                };
                let node = Node {
                    id: member.id,
                    space: space.id,
                    kind: member.kind,
                    parent: member.original_parent,
                    path,
                    title: member.title.clone(),
                    size,
                    modified: manifest.trashed_at,
                    created_by: member.created_by,
                    target: existing.get(&member.id).and_then(|n| n.target),
                    trash: Some(TrashState {
                        trashed_at: manifest.trashed_at,
                        trashed_by: manifest.trashed_by,
                        explicitly_trashed: member.id == top.id,
                        entry: *entry,
                        original_path: member.original_path.clone(),
                        original_parent: member.original_parent,
                    }),
                };
                if !existing.contains_key(&node.id) {
                    report.added += 1;
                }
                self.index.put_node(&node).await?;
                let _ = live.insert(node.id);
            }
        }

        for id in existing.keys() {
            if !live.contains(id) {
                self.index.remove_node(*id).await?;
                report.removed += 1;
            }
        }
        for (node, text) in &found {
            if let Some(text) = text {
                self.reindex_links(node, text).await?;
            }
        }
        Ok(())
    }

    /// Reads every node at or beneath the given blobs of a space: pages,
    /// folders, books, collections and shortcuts. Files inside books and
    /// collections belong to them and are not nodes. Pages come with their
    /// text, for link indexing.
    async fn discover(
        &self,
        space: &Space,
        blobs: &Blobs,
        known: &HashSet<BlobPath>,
        seen: &mut HashSet<ObjectId>,
        fresh: bool,
        report: &mut ScanReport,
    ) -> DriveResult<Vec<(Node, Option<String>)>> {
        let mut taken = Taken { ids: seen, fresh };
        let mut out = Vec::new();
        let now = self.clock.now();
        // Files already indexed at their path go first, so when two files
        // share an ID the one that had it keeps it and the newcomer is
        // re-identified.
        let (first, rest): (Vec<_>, Vec<_>) =
            blobs.iter().partition(|(p, _)| known.contains(*p));
        for (path, size) in first.into_iter().chain(rest) {
            if layout::is_state(path) || inside_unit(path, &space.root) {
                continue;
            }
            if path.parent().as_ref() == Some(&space.root)
                && path.file_name() == layout::FOLDER_FILE
            {
                continue;
            }
            let Some((kind, node_path)) = layout::classify(path) else {
                continue;
            };
            let read = self
                .read_node(space, kind, path, &node_path, &mut taken, report)
                .await;
            let (id, title, target, text) = match read {
                Ok(read) => read,
                Err(err) => {
                    tracing::warn!(%path, error = %err, "skipping unreadable \
                        file");
                    report.skipped += 1;
                    continue;
                }
            };
            let size = if kind.is_dir() && kind != NodeKind::Folder {
                blobs
                    .range(node_path.clone()..)
                    .take_while(|(p, _)| p.starts_with(&node_path))
                    .map(|(_, s)| *s)
                    .sum()
            } else if kind == NodeKind::Folder {
                0
            } else {
                *size
            };
            out.push((
                Node {
                    id,
                    space: space.id,
                    kind,
                    parent: None,
                    path: node_path,
                    title,
                    size,
                    modified: now,
                    created_by: None,
                    target,
                    trash: None,
                },
                text,
            ));
        }
        Ok(out)
    }

    /// Reads a node's identity file, giving it a fresh ID when it has none
    /// or its ID is already taken.
    async fn read_node(
        &self,
        space: &Space,
        kind: NodeKind,
        file: &BlobPath,
        node_path: &BlobPath,
        seen: &mut Taken<'_>,
        report: &mut ScanReport,
    ) -> DriveResult<(ObjectId, String, Option<ObjectId>, Option<String>)> {
        let _ = space;
        let fallback = layout::title_from_path(kind, node_path);
        match kind {
            NodeKind::Folder => {
                let folder: FolderFile = self.read_yaml(file).await?;
                let id = self
                    .claim(folder.id, seen, file, report, |text, id| {
                        let mut f = folder.clone();
                        f.id = id;
                        let _ = text;
                        crate::files::to_yaml(&f)
                    })
                    .await?;
                Ok((id, fallback, None, None))
            }
            NodeKind::Shortcut => {
                let shortcut: ShortcutFile = self.read_yaml(file).await?;
                let id = self
                    .claim(shortcut.id, seen, file, report, |_, id| {
                        let mut s = shortcut.clone();
                        s.id = id;
                        crate::files::to_yaml(&s)
                    })
                    .await?;
                Ok((id, shortcut.title.clone(), Some(shortcut.target), None))
            }
            NodeKind::Book | NodeKind::Collection => {
                let (text, _) = self.read_text(file).await?;
                let id = frontmatter::get(&text, "id")
                    .and_then(|id| id.parse().ok());
                let title =
                    frontmatter::get(&text, "title").unwrap_or(fallback);
                let id = self
                    .claim_or_new(id, seen, file, report, &text, |text, id| {
                        Ok(frontmatter::set(text, "id", &id.to_string()))
                    })
                    .await?;
                Ok((id, title, None, None))
            }
            NodeKind::Page => {
                let (text, _) = self.read_text(file).await?;
                let original = frontmatter::get_md(&text, "id")
                    .and_then(|id| id.parse().ok());
                let title = frontmatter::get_md(&text, "title")
                    .map(|t| t.trim().to_owned())
                    .filter(|t| !t.is_empty())
                    .unwrap_or(fallback);
                let id = self
                    .claim_or_new(
                        original,
                        seen,
                        file,
                        report,
                        &text,
                        |text, id| {
                            Ok(frontmatter::set_md(text, "id", &id.to_string()))
                        },
                    )
                    .await?;
                let text = if original == Some(id) {
                    text
                } else {
                    self.read_text(file).await?.0
                };
                Ok((id, title, None, Some(text)))
            }
        }
    }

    /// Whether `id` is still free for a file being indexed, taking it if
    /// so. With `fresh` (a copy or an import, whose files arrive with IDs
    /// someone else chose), an ID the index already holds anywhere is
    /// taken too: an imported file naming another person's page must not
    /// move that page's entry into the import (RT-037).
    async fn free(
        &self,
        id: ObjectId,
        seen: &mut Taken<'_>,
    ) -> DriveResult<bool> {
        if seen.fresh && self.index.node(id).await?.is_some() {
            return Ok(false);
        }
        Ok(seen.ids.insert(id))
    }

    /// Takes `id` for this file, or rewrites the file with a fresh ID if
    /// another file already holds it.
    async fn claim(
        &self,
        id: ObjectId,
        seen: &mut Taken<'_>,
        file: &BlobPath,
        report: &mut ScanReport,
        rewrite: impl FnOnce(&str, ObjectId) -> DriveResult<String> + Send,
    ) -> DriveResult<ObjectId> {
        if self.free(id, seen).await? {
            return Ok(id);
        }
        let fresh = ObjectId::generate();
        let text = rewrite("", fresh)?;
        let _ = self.put_file(file, text).await?;
        let _ = seen.ids.insert(fresh);
        report.reidentified += 1;
        Ok(fresh)
    }

    /// Like [`Self::claim`] for files whose ID may be missing; the rewrite
    /// edits the existing text.
    async fn claim_or_new(
        &self,
        id: Option<ObjectId>,
        seen: &mut Taken<'_>,
        file: &BlobPath,
        report: &mut ScanReport,
        text: &str,
        rewrite: impl FnOnce(&str, ObjectId) -> DriveResult<String> + Send,
    ) -> DriveResult<ObjectId> {
        if let Some(id) = id
            && self.free(id, seen).await?
        {
            return Ok(id);
        }
        let fresh = ObjectId::generate();
        let new = rewrite(text, fresh)?;
        let blob = self.store.head(file).await?;
        let _ = self
            .store
            .put(file, Bytes::from(new), Precondition::IfMatch(blob.etag))
            .await?;
        let _ = seen.ids.insert(fresh);
        report.reidentified += 1;
        Ok(fresh)
    }

    async fn scan_versions(&self, blobs: &Blobs) -> DriveResult<()> {
        for (path, size) in blobs {
            let mut parts = path.segments();
            let (
                Some(layout::STATE_DIR),
                Some("versions"),
                Some(doc),
                Some(name),
                None,
            ) = (
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
                parts.next(),
            )
            else {
                continue;
            };
            let (Ok(doc), Some(Ok(version))) = (
                doc.parse::<ObjectId>(),
                name.strip_suffix(".md").map(str::parse::<VersionId>),
            ) else {
                continue;
            };
            let Some(node) = self.index.node(doc).await? else {
                continue;
            };
            let (_, sidecar) = layout::version_paths(doc, version)?;
            let author = self
                .read_version_file(&sidecar)
                .await
                .map_or(vellum_kernel::Actor::System, |file| file.author);
            self.index
                .put_version(&VersionRecord {
                    id: version,
                    doc,
                    space: node.space,
                    size: *size,
                    author,
                })
                .await?;
        }
        Ok(())
    }

    /// The directories of a space's live folders and pages, for finding
    /// parents.
    async fn container_paths(
        &self,
        space: SpaceId,
    ) -> DriveResult<HashMap<BlobPath, ObjectId>> {
        let nodes = self.index.nodes(space).await?;
        Ok(containers(nodes.iter().filter(|n| n.is_live())))
    }

    async fn read_yaml<T: serde::de::DeserializeOwned>(
        &self,
        path: &BlobPath,
    ) -> DriveResult<T> {
        let blob = self.store.get(path).await?;
        from_yaml(path, &blob.bytes)
    }
}

/// The directories that hold other nodes: each folder's own, and the one
/// beside each page (D168). A folder wins over a page of the same name.
fn containers<'a>(
    nodes: impl Iterator<Item = &'a Node>,
) -> HashMap<BlobPath, ObjectId> {
    let mut out = HashMap::new();
    let mut folders = Vec::new();
    for node in nodes {
        match node.kind {
            NodeKind::Folder => folders.push((node.path.clone(), node.id)),
            NodeKind::Page => {
                if let Some(dir) = layout::page_dir(&node.path) {
                    let _ = out.insert(dir, node.id);
                }
            }
            NodeKind::Book | NodeKind::Collection | NodeKind::Shortcut => {}
        }
    }
    out.extend(folders);
    out
}

/// The nearest folder or page above `path`, if any.
fn parent_of(
    path: &BlobPath,
    holders: &HashMap<BlobPath, ObjectId>,
) -> Option<ObjectId> {
    let mut dir = path.parent();
    while let Some(d) = dir {
        if let Some(id) = holders.get(&d) {
            return Some(*id);
        }
        dir = d.parent();
    }
    None
}

/// True for files inside a book or collection, other than its manifest.
fn inside_unit(path: &BlobPath, root: &BlobPath) -> bool {
    let mut dir = path.parent();
    while let Some(d) = dir {
        if &d == root {
            return false;
        }
        let name = d.file_name();
        if name.ends_with(NodeKind::Book.suffix())
            || name.ends_with(NodeKind::Collection.suffix())
        {
            let manifest = path.parent().as_ref() == Some(&d)
                && matches!(
                    path.file_name(),
                    layout::BOOK_FILE | layout::COLLECTION_FILE
                );
            return !manifest;
        }
        dir = d.parent();
    }
    false
}
