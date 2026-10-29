// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Saving a blob that may have changed underneath the writer.
//!
//! [`save`] writes conditionally on the version the writer started from.
//! If someone else (an external editor, a sync client, another replica)
//! changed the blob meanwhile, it three-way merges the two versions and
//! retries. If the merge conflicts, the stored version stays in place and
//! the writer's version goes to a sibling conflict copy,
//! `Title (conflict 2026-10-29 alice).md`, so nothing is ever lost (D12).

use bytes::Bytes;

use crate::{
    error::{StoreError, StoreResult},
    merge::{MergeKind, Merged, merge3},
    path::BlobPath,
    store::{BlobMeta, BlobStore, Etag, Precondition},
};

/// How many times a save re-merges when the blob keeps moving.
const MAX_ATTEMPTS: usize = 8;

/// The version a writer started from.
#[derive(Clone, Debug)]
pub struct Base {
    /// Content the writer edited.
    pub bytes: Bytes,
    /// Its etag, or `None` if the writer is creating the blob.
    pub etag: Option<Etag>,
}

/// Who is saving, for naming a conflict copy.
pub struct Writer<'a> {
    /// Display name of the user whose save conflicted.
    pub user: &'a str,
    /// The user's local date, `YYYY-MM-DD`.
    pub date: &'a str,
    /// Gives the conflict copy a fresh identity (a new front-matter `id`),
    /// since identity is the ULID and two files must never share one.
    pub reidentify: &'a (dyn Fn(&[u8]) -> Bytes + Send + Sync),
}

impl std::fmt::Debug for Writer<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Writer")
            .field("user", &self.user)
            .field("date", &self.date)
            .finish()
    }
}

/// What a save did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Saved {
    /// Nobody else had changed the blob; it now holds the writer's bytes.
    Written(BlobMeta),
    /// Concurrent changes were merged cleanly with the writer's.
    Merged(BlobMeta),
    /// The changes conflicted: `kept` is the stored version, left as is,
    /// and `copy` holds the writer's version.
    Conflicted {
        /// The blob at the original path.
        kept: BlobMeta,
        /// The conflict copy.
        copy: BlobMeta,
    },
}

/// Saves `ours` at `path`, merging with any change made since `base`.
///
/// # Errors
/// Returns store errors other than precondition failures, which are
/// handled by merging, or `Backend` if the blob keeps changing.
#[tracing::instrument(skip_all, fields(path = %path, outcome), err)]
pub async fn save(
    store: &dyn BlobStore,
    path: &BlobPath,
    base: Base,
    ours: Bytes,
    writer: &Writer<'_>,
) -> StoreResult<Saved> {
    let (mut base, mut ours, mut merged) = (base, ours, false);
    for _ in 0..MAX_ATTEMPTS {
        let cond = base
            .etag
            .clone()
            .map_or(Precondition::IfAbsent, Precondition::IfMatch);
        match store.put(path, ours.clone(), cond).await {
            Ok(meta) => {
                record(if merged { "merged" } else { "written" });
                return Ok(if merged {
                    Saved::Merged(meta)
                } else {
                    Saved::Written(meta)
                });
            }
            Err(StoreError::PreconditionFailed { current: None, .. }) => {
                // Deleted meanwhile: an edit outranks a delete, so recreate.
                base = Base {
                    bytes: Bytes::new(),
                    etag: None,
                };
            }
            Err(StoreError::PreconditionFailed { .. } | StoreError::AlreadyExists(_)) => {
                let theirs = match store.get(path).await {
                    Ok(theirs) => theirs,
                    Err(StoreError::NotFound(_)) => continue,
                    Err(e) => return Err(e),
                };
                let kind = MergeKind::for_path(path);
                let Some(text) = three_way(kind, &base.bytes, &ours, &theirs.bytes) else {
                    let copy = write_copy(store, path, &ours, writer).await?;
                    record("conflicted");
                    return Ok(Saved::Conflicted {
                        kept: theirs.meta,
                        copy,
                    });
                };
                (ours, merged) = (text, true);
                base = Base {
                    bytes: theirs.bytes,
                    etag: Some(theirs.meta.etag),
                };
            }
            Err(e) => return Err(e),
        }
    }
    Err(StoreError::Backend(format!(
        "{path} kept changing; gave up after {MAX_ATTEMPTS} merges"
    )))
}

fn record(outcome: &str) {
    let _ = tracing::Span::current().record("outcome", outcome);
}

/// Merges the three versions; `None` is a conflict. Binary content only
/// merges when one side left it unchanged.
fn three_way(kind: MergeKind, base: &[u8], ours: &Bytes, theirs: &Bytes) -> Option<Bytes> {
    if ours == theirs || base == theirs.as_ref() {
        return Some(ours.clone());
    }
    if base == ours.as_ref() {
        return Some(theirs.clone());
    }
    match (
        str::from_utf8(base),
        str::from_utf8(ours),
        str::from_utf8(theirs),
    ) {
        (Ok(b), Ok(o), Ok(t)) => match merge3(kind, b, o, t) {
            Merged::Clean(text) => Some(Bytes::from(text)),
            Merged::Conflict => None,
        },
        _ => None,
    }
}

async fn write_copy(
    store: &dyn BlobStore,
    path: &BlobPath,
    ours: &[u8],
    writer: &Writer<'_>,
) -> StoreResult<BlobMeta> {
    let bytes = (writer.reidentify)(ours);
    for n in 1..=MAX_ATTEMPTS {
        let copy = conflict_path(path, writer.date, writer.user, n)?;
        match store
            .put(&copy, bytes.clone(), Precondition::IfAbsent)
            .await
        {
            Err(StoreError::AlreadyExists(_)) => {}
            other => return other,
        }
    }
    Err(StoreError::AlreadyExists(path.clone()))
}

/// The sibling path for a conflict copy: `Title (conflict <date> <user>).md`,
/// with ` 2`, ` 3`, … appended inside the brackets on collision.
///
/// # Errors
/// Returns [`StoreError::InvalidPath`] if the name becomes invalid.
pub fn conflict_path(path: &BlobPath, date: &str, user: &str, n: usize) -> StoreResult<BlobPath> {
    let name = path.file_name();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (name, String::new()),
    };
    let user: String = user
        .chars()
        .map(|c| if c == '/' || c.is_control() { '-' } else { c })
        .collect();
    let nth = if n > 1 {
        format!(" {n}")
    } else {
        String::new()
    };
    let file = format!("{stem} (conflict {date} {}{nth}){ext}", user.trim());
    path.parent()
        .map_or_else(|| BlobPath::new(file.clone()), |dir| dir.join(&file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_copies_are_siblings() -> StoreResult<()> {
        let p = BlobPath::new("drives/users/alice/Q3 plan.md")?;
        let copy = conflict_path(&p, "2026-10-29", "alice", 1)?;
        assert_eq!(
            copy.as_str(),
            "drives/users/alice/Q3 plan (conflict 2026-10-29 alice).md"
        );
        let second = conflict_path(&p, "2026-10-29", "a/b", 2)?;
        assert_eq!(second.file_name(), "Q3 plan (conflict 2026-10-29 a-b 2).md");
        let bare = conflict_path(&BlobPath::new(".vellum-folder")?, "2026-10-29", "bo", 1)?;
        assert_eq!(bare.as_str(), ".vellum-folder (conflict 2026-10-29 bo)");
        Ok(())
    }
}
