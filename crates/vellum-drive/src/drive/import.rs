// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Writing an import into a drive (D230).
//!
//! An importer (Notion, Confluence, Word and the rest, in `vellum-export`)
//! turns an upload into files with paths relative to one folder. This
//! writes them into a new folder of the drive, so an import never mixes
//! with what is already there, and indexes the result the way a copy is
//! indexed: identity comes from the IDs inside the files.

use std::collections::BTreeSet;

use bytes::Bytes;
use vellum_kernel::{Actor, ObjectId, SpaceId};
use vellum_store::{BlobPath, Precondition};

use super::Drive;
use crate::error::{DriveError, DriveResult};
use crate::files::{FolderFile, to_yaml};
use crate::layout;
use crate::model::{ActivityKind, Node, NodeKind};
use crate::ports::Permission;

/// Most files one import may write.
pub const MAX_IMPORT_FILES: usize = 20_000;

/// What an import wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// The new folder holding everything imported.
    pub folder: Node,
    /// The nodes indexed inside it (pages, folders, collections).
    pub nodes: usize,
    /// Files written, attachments and folder files included.
    pub files: usize,
}

impl Drive {
    /// Checks that `actor` may import into `parent` of `space`, so a
    /// route can refuse before reading a large upload.
    ///
    /// # Errors
    /// `SpaceNotFound`, `NotFound` or `PermissionDenied`.
    pub async fn may_import(
        &self,
        actor: &Actor,
        space: SpaceId,
        parent: Option<ObjectId>,
    ) -> DriveResult<()> {
        let space = self.space(space).await?;
        self.authorize(
            actor,
            space.id,
            parent.unwrap_or_else(|| space.node_id()),
            Permission::Edit,
        )
        .await
    }

    /// Writes `files` (relative path, bytes) into a new folder called
    /// `name` in `parent` of `space` (`None`: the space root).
    ///
    /// Paths must be relative and stay inside the folder; they may not
    /// name Vellum's own files (`.vellum…`, `.attachments`). Every
    /// directory that is not a page's nested pages, a book or a
    /// collection becomes a folder.
    ///
    /// # Errors
    /// `PermissionDenied` without edit rights on the parent, `Invalid` for
    /// a bad path or too many files, `QuotaExceeded`.
    #[tracing::instrument(name = "drive.import", skip_all, fields(space = %space, files = files.len()), err)]
    pub async fn import(
        &self,
        actor: &Actor,
        space: SpaceId,
        parent: Option<ObjectId>,
        name: &str,
        files: Vec<(String, Bytes)>,
    ) -> DriveResult<Imported> {
        self.observe("import", async {
            let space = self.space(space).await?;
            self.authorize(
                actor,
                space.id,
                parent.unwrap_or_else(|| space.node_id()),
                Permission::Edit,
            )
            .await?;
            if files.len() > MAX_IMPORT_FILES {
                return Err(DriveError::Invalid(format!(
                    "an import holds at most {MAX_IMPORT_FILES} files"
                )));
            }
            for (path, _) in &files {
                check_path(path)?;
            }
            let total: u64 = files.iter().map(|(_, b)| b.len() as u64).sum();
            self.ensure_quota(&space, total).await?;

            let dir = self.dir_of(&space, parent).await?;
            let title = name.trim();
            let title = if title.is_empty() { "Import" } else { title };
            let dest = self.free_path(&dir, NodeKind::Folder, title, None).await?;
            let folders = folder_dirs(files.iter().map(|(p, _)| p.as_str()));
            let mut written = 0_usize;
            for folder in std::iter::once(String::new()).chain(folders) {
                let at = if folder.is_empty() {
                    dest.clone()
                } else {
                    dest.join(&folder)?
                };
                let file = FolderFile {
                    id: ObjectId::generate(),
                    icon: None,
                    space: None,
                };
                let _ = self
                    .store
                    .put(
                        &at.join(layout::FOLDER_FILE)?,
                        Bytes::from(to_yaml(&file)?),
                        Precondition::IfAbsent,
                    )
                    .await?;
                written += 1;
            }
            for (path, bytes) in files {
                let _ = self
                    .store
                    .put(&dest.join(&path)?, bytes, Precondition::IfAbsent)
                    .await?;
                written += 1;
            }
            let nodes = self
                .index_tree(&space, &dest, true, parent, Self::user_of(actor))
                .await?;
            let top = nodes
                .iter()
                .find(|n| n.path == dest)
                .cloned()
                .ok_or_else(|| DriveError::Invalid("the import could not be indexed".into()))?;
            if let Some(user) = Self::user_of(actor) {
                self.access.grant_owner(user, top.space, top.id).await?;
                self.index.touch(user, top.id, top.modified).await?;
            }
            let ids: Vec<ObjectId> = nodes.iter().map(|n| n.id).collect();
            self.access.tree_changed(space.id, &ids).await?;
            self.record(actor, &top, ActivityKind::Created).await?;
            tracing::info!(folder = %top.id, nodes = nodes.len(), files = written, "import written");
            Ok(Imported {
                nodes: nodes.len().saturating_sub(1),
                folder: top,
                files: written,
            })
        })
        .await
    }
}

/// Refuses a path that leaves the folder or names Vellum's own files.
fn check_path(path: &str) -> DriveResult<()> {
    let bad = |why: &str| Err(DriveError::Invalid(format!("import path {path:?}: {why}")));
    if path.starts_with('/') {
        return bad("must be relative");
    }
    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return bad("empty or relative segment");
        }
        if segment.starts_with(".vellum")
            || segment == vellum_store::layout::ATTACHMENTS_DIR
            || segment == ".snapshots"
        {
            return bad("names a Vellum file");
        }
    }
    // The store's own rules (Windows names, `.git`, length) apply too.
    match BlobPath::new(path) {
        Ok(_) => Ok(()),
        Err(e) => bad(&e.to_string()),
    }
}

/// The directories among `paths` that become folders: every ancestor
/// directory except a page's nested pages (`Title/` beside `Title.md`,
/// D168) and anything inside a book or collection. Sorted, so a parent
/// comes before its children.
fn folder_dirs<'a>(paths: impl Iterator<Item = &'a str> + Clone) -> Vec<String> {
    let pages: BTreeSet<&str> = paths
        .clone()
        .filter_map(|p| p.strip_suffix(".md"))
        .collect();
    let mut dirs = BTreeSet::new();
    for path in paths {
        let mut prefix = String::new();
        let mut segments = path.split('/').peekable();
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                break;
            }
            if segment.ends_with(NodeKind::Book.suffix())
                || segment.ends_with(NodeKind::Collection.suffix())
            {
                break;
            }
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            if !pages.contains(prefix.as_str()) {
                let _ = dirs.insert(prefix.clone());
            }
        }
    }
    dirs.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_skip_page_children_and_units() {
        let paths = [
            "Team.md",
            "Team/Onboarding.md",
            "Guides/Setup.md",
            "Guides/assets/shot.png",
            "Tasks.collection/schema.yaml",
            "Tasks.collection/rows/a.md",
        ];
        assert_eq!(
            folder_dirs(paths.iter().copied()),
            vec!["Guides".to_owned(), "Guides/assets".to_owned()]
        );
    }

    #[test]
    fn paths_stay_inside_and_off_vellum_files() {
        for bad in [
            "../x.md",
            "/abs.md",
            "a//b.md",
            "a/./b.md",
            ".vellum/trash/x",
            "p/.attachments/x.png",
            "a/.vellum-folder.yaml",
            "Book.book/.snapshots/s.json",
            "a/.git/config",
        ] {
            assert!(check_path(bad).is_err(), "{bad} accepted");
        }
        assert!(check_path("Guides/Setup.md").is_ok());
    }
}
