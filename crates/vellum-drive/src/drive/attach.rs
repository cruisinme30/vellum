// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Files uploaded to a page (D211).
//!
//! A page `Plan.md` keeps its uploaded files in `Plan/.attachments/`, in
//! the directory beside it that also holds its nested pages (D168). So a
//! move, copy, trash or restore of the page carries its files, and the
//! folder still reads naturally on disk. The files are not in the index:
//! a page's files are listed from the store. Callers check uploads first
//! (`vellum_security::upload::check` in `vellum serve`); the drive only
//! stores what it is given, under the page's permissions.

use bytes::Bytes;
use vellum_kernel::{Actor, ObjectId};
use vellum_store::layout::{ATTACHMENTS_DIR, MAX_NAME_BYTES, in_attachments};
use vellum_store::{BlobPath, Precondition, StoreError};

use super::Drive;
use crate::error::{DriveError, DriveResult};
use crate::model::{Node, NodeKind, Space};
use crate::ports::Permission;

/// The most files one page may hold.
pub const MAX_ATTACHMENTS: usize = 1_000;

/// A file uploaded to a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    /// The page that holds it.
    pub page: ObjectId,
    /// Its file name, unique within the page.
    pub name: String,
    /// Its size in bytes, as stored.
    pub size: u64,
}

impl Drive {
    /// The live page `id`, when `actor` may read its files (`write`:
    /// change them). Upload routes call this before reading a request
    /// body, so a caller who may not write sends nothing big.
    ///
    /// # Errors
    /// `NotFound`, `Invalid` for an item that is not a page,
    /// `PermissionDenied`.
    pub async fn attachment_page(
        &self,
        actor: &Actor,
        id: ObjectId,
        write: bool,
    ) -> DriveResult<Node> {
        let node = self.live_node(id).await?;
        let permission = if write {
            Permission::Edit
        } else {
            Permission::View
        };
        self.authorize(actor, node.space, node.id, permission)
            .await?;
        if node.kind != NodeKind::Page {
            return Err(DriveError::Invalid("only pages hold files".into()));
        }
        Ok(node)
    }

    /// Stores `bytes` as a file of the page `page`, named `name` or, when
    /// that is taken, `name (2)` and so on.
    ///
    /// # Errors
    /// As [`Self::attachment_page`], plus `Invalid` for a bad name or a
    /// full page, `QuotaExceeded`.
    #[tracing::instrument(name = "drive.attach", skip_all, fields(page = %page, bytes = bytes.len()), err)]
    pub async fn attach(
        &self,
        actor: &Actor,
        page: ObjectId,
        name: &str,
        bytes: Bytes,
    ) -> DriveResult<Attachment> {
        self.observe("attach", async {
            let node = self.attachment_page(actor, page, true).await?;
            let space = self.space(node.space).await?;
            let dir = self.files_dir(&node).await?;
            if self.blobs_under(&dir).await?.len() >= MAX_ATTACHMENTS {
                return Err(DriveError::Invalid(format!(
                    "a page holds at most {MAX_ATTACHMENTS} files"
                )));
            }
            self.ensure_file_quota(&space, bytes.len() as u64).await?;
            let candidates = names(name)?;
            // A text file is checked like page text (secret scanning, D301).
            if std::str::from_utf8(&bytes).is_ok() {
                let path = file_path(&dir, name).unwrap_or_else(|_| dir.clone());
                self.check_content(actor, node.space, &path, &bytes).await?;
            }
            for candidate in candidates.take(MAX_ATTACHMENTS + 1) {
                let path = file_path(&dir, &candidate)?;
                match self
                    .store
                    .put(&path, bytes.clone(), Precondition::IfAbsent)
                    .await
                {
                    Ok(meta) => {
                        return Ok(Attachment {
                            page,
                            name: candidate,
                            size: meta.size,
                        });
                    }
                    Err(StoreError::AlreadyExists(_)) => {}
                    Err(err) => return Err(err.into()),
                }
            }
            Err(DriveError::AlreadyExists(name.to_owned()))
        })
        .await
    }

    /// The files of the page `page`, by name.
    ///
    /// # Errors
    /// As [`Self::attachment_page`].
    pub async fn attachments(&self, actor: &Actor, page: ObjectId) -> DriveResult<Vec<Attachment>> {
        self.observe("attachments", async {
            let node = self.attachment_page(actor, page, false).await?;
            let dir = self.files_dir(&node).await?;
            let mut files: Vec<Attachment> = self
                .blobs_under(&dir)
                .await?
                .into_iter()
                .filter(|(path, _)| path.parent().as_ref() == Some(&dir))
                .map(|(path, size)| Attachment {
                    page,
                    name: path.file_name().to_owned(),
                    size,
                })
                .collect();
            files.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(files)
        })
        .await
    }

    /// One file of the page `page` and its bytes.
    ///
    /// # Errors
    /// As [`Self::attachment_page`]; `NotFound` when the page has no such
    /// file.
    pub async fn attachment(
        &self,
        actor: &Actor,
        page: ObjectId,
        name: &str,
    ) -> DriveResult<(Attachment, Bytes)> {
        self.observe("attachment", async {
            let node = self.attachment_page(actor, page, false).await?;
            let path = file_path(&self.files_dir(&node).await?, name)?;
            match self.store.get(&path).await {
                Ok(blob) => Ok((
                    Attachment {
                        page,
                        name: name.to_owned(),
                        size: blob.meta.size,
                    },
                    blob.bytes,
                )),
                Err(StoreError::NotFound(_)) => Err(DriveError::NotFound(page)),
                Err(err) => Err(err.into()),
            }
        })
        .await
    }

    /// Deletes a file of the page `page`. The page's versions keep any
    /// link to it; the link then shows a missing file.
    ///
    /// # Errors
    /// As [`Self::attachment_page`]; `NotFound` when the page has no such
    /// file.
    #[tracing::instrument(name = "drive.detach", skip_all, fields(page = %page), err)]
    pub async fn detach(&self, actor: &Actor, page: ObjectId, name: &str) -> DriveResult<()> {
        self.observe("detach", async {
            let node = self.attachment_page(actor, page, true).await?;
            let path = file_path(&self.files_dir(&node).await?, name)?;
            match self.store.delete(&path, Precondition::None).await {
                Ok(()) => Ok(()),
                Err(StoreError::NotFound(_)) => Err(DriveError::NotFound(page)),
                Err(err) => Err(err.into()),
            }
        })
        .await
    }

    /// `Plan/.attachments/` for the page `Plan.md`.
    async fn files_dir(&self, page: &Node) -> DriveResult<BlobPath> {
        // A folder made outside Vellum can take the page's directory
        // (D168); its files then have nowhere to go.
        let dir = self
            .children_dir(page.kind, &page.path)
            .await?
            .ok_or_else(|| DriveError::Invalid("this page cannot hold files".into()))?;
        Ok(dir.join(ATTACHMENTS_DIR)?)
    }

    /// Fails when `bytes` more would take the space past its quota. The
    /// index does not count uploaded files, so they are added up from the
    /// store, and only for a space that has a quota.
    async fn ensure_file_quota(&self, space: &Space, bytes: u64) -> DriveResult<()> {
        let Some(quota) = space.settings.quota_bytes else {
            return Ok(());
        };
        let files: u64 = self
            .blobs_under(&space.root)
            .await?
            .iter()
            .filter(|(path, _)| in_attachments(path))
            .map(|(_, size)| *size)
            .sum();
        let used = self.index.usage(space.id).await?.saturating_add(files);
        if used.saturating_add(bytes) > quota {
            return Err(DriveError::QuotaExceeded {
                used,
                quota,
                needed: bytes,
            });
        }
        Ok(())
    }
}

/// The path of the file `name` in `dir`, refusing anything that is not a
/// plain file name.
fn file_path(dir: &BlobPath, name: &str) -> DriveResult<BlobPath> {
    let bad = || DriveError::Invalid(format!("{name:?} is not a valid file name"));
    if name.is_empty() || name.starts_with('.') || name.contains(['/', '\\']) {
        return Err(bad());
    }
    let path = dir.join(name).map_err(|_| bad())?;
    if path.parent().as_ref() == Some(dir) {
        Ok(path)
    } else {
        Err(bad())
    }
}

/// `name`, then `stem (2).ext`, `stem (3).ext` and so on, each short
/// enough for every backend.
fn names(name: &str) -> DriveResult<impl Iterator<Item = String>> {
    if name.is_empty() || name.len() > MAX_NAME_BYTES {
        return Err(DriveError::Invalid(format!(
            "a file name has 1 to {MAX_NAME_BYTES} bytes"
        )));
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem.to_owned(), format!(".{ext}")),
        _ => (name.to_owned(), String::new()),
    };
    let first = std::iter::once(name.to_owned());
    let rest = (2_usize..).map(move |n| {
        let suffix = format!(" ({n}){ext}");
        let mut stem = stem.clone();
        while stem.len() + suffix.len() > MAX_NAME_BYTES {
            let _ = stem.pop();
        }
        format!("{stem}{suffix}")
    });
    Ok(first.chain(rest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn taken_names_get_a_number() {
        let got: Vec<String> = names("chart.png")
            .unwrap_or_else(|e| unreachable!("{e}"))
            .take(3)
            .collect();
        assert_eq!(got, ["chart.png", "chart (2).png", "chart (3).png"]);
        let long = "a".repeat(MAX_NAME_BYTES);
        let second = names(&long)
            .unwrap_or_else(|e| unreachable!("{e}"))
            .nth(1)
            .unwrap_or_default();
        assert!(second.len() <= MAX_NAME_BYTES && second.ends_with(" (2)"));
        assert!(names("").is_err());
    }

    #[test]
    fn only_plain_names_are_files() {
        let dir = BlobPath::new("d/Plan/.attachments").unwrap_or_else(|e| unreachable!("{e}"));
        assert!(file_path(&dir, "chart.png").is_ok());
        for bad in ["", ".env", "../Plan.md", "a/b.png", "a\\b.png", "CON"] {
            assert!(file_path(&dir, bad).is_err(), "{bad:?}");
        }
    }
}
