// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Books as one person sees them.
//!
//! A book narrows access per section with `restrict_to` in `book.yaml`,
//! which the drive's own permission checks know nothing about. Anything
//! that hands a book's files to a person (a copy they will own, or its
//! `book.yaml`) goes through [`Drive::book_view`] first. That view leaves
//! out the sections they may not see, without moving any other file.
//! Snapshots (`.snapshots/*.json`) carry the full text of every section,
//! so they never leave the book.

use std::collections::{BTreeMap, BTreeSet};

use bytes::Bytes;
use vellum_books::manifest::Principal;
use vellum_books::publish::Audience;
use vellum_books::{Book, IdSource, NodeId};
use vellum_kernel::{Actor, ObjectId};
use vellum_store::BlobPath;

use super::Drive;
use crate::error::{DriveError, DriveResult};
use crate::layout;

/// The book's settings file.
const MANIFEST: &str = "book.yaml";
/// The book's table of contents.
const SUMMARY: &str = "SUMMARY.md";
/// Tagged snapshots, holding every section's full text.
const SNAPSHOTS: &str = ".snapshots";

/// The files of a book as one person may see it, by path.
pub(super) type BookFiles = BTreeMap<BlobPath, Bytes>;

/// Fresh section IDs for chapters that lack one; the load only needs
/// them to hold the book in memory.
struct FreshIds;

impl IdSource for FreshIds {
    fn next_id(&mut self) -> NodeId {
        loop {
            if let Ok(id) = NodeId::parse(&ObjectId::generate().to_string()) {
                return id;
            }
        }
    }
}

/// The principals a person holds, for section restrictions. `None` for
/// Vellum itself and outside edits, which see everything.
///
/// Group membership is not known to the drive yet, so a person allowed
/// only through a group sees less, never more.
fn principals(actor: &Actor) -> DriveResult<Option<BTreeSet<Principal>>> {
    let user = match actor {
        Actor::User { user } => user,
        Actor::Model { on_behalf_of } => on_behalf_of,
        _ => return Ok(None),
    };
    let principal = Principal::parse(&format!("user:{user}"))
        .map_err(|err| DriveError::Invalid(format!("principal for {user}: {err}")))?;
    Ok(Some(BTreeSet::from([principal])))
}

/// True for files a book copy never carries: snapshots and build output.
pub(super) fn never_copied(book: &BlobPath, path: &BlobPath) -> bool {
    relative(book, path).is_some_and(|rel| {
        rel.split('/')
            .next()
            .is_some_and(|first| first == SNAPSHOTS || first == ".mdbook")
    })
}

/// True for the files a book view replaces: `book.yaml` and every
/// Markdown file of the book.
pub(super) fn from_view(book: &BlobPath, path: &BlobPath) -> bool {
    relative(book, path).is_some_and(|rel| rel == MANIFEST || layout::has_ext(rel, "md"))
}

impl Drive {
    /// The book at `root` as `actor` may see it: its `book.yaml`,
    /// `SUMMARY.md` and the files of the sections they may see. Other
    /// files of the book are not included.
    ///
    /// # Errors
    /// `Invalid` when the book cannot be loaded (refusing is the safe
    /// answer), or a store failure.
    pub(super) async fn book_view(&self, actor: &Actor, root: &BlobPath) -> DriveResult<BookFiles> {
        let mut texts: BTreeMap<String, String> = BTreeMap::new();
        for (path, _) in self.blobs_under(root).await? {
            if never_copied(root, &path) || !from_view(root, &path) {
                continue;
            }
            let Some(rel) = relative(root, &path).map(str::to_owned) else {
                continue;
            };
            let blob = self.store.get(&path).await?;
            let text = String::from_utf8(blob.bytes.to_vec()).map_err(|_| unreadable(root))?;
            let _ = texts.insert(rel, text);
        }
        let manifest = texts.get(MANIFEST).ok_or_else(|| unreadable(root))?;
        let summary = texts.get(SUMMARY).map_or("", String::as_str);
        let (book, _repairs) = Book::load(
            manifest,
            summary,
            |chapter| texts.get(chapter.as_str()).cloned(),
            &mut FreshIds,
        )
        .map_err(|_| unreadable(root))?;
        let view = match principals(actor)? {
            Some(principals) => book
                .published_view(&Audience::Members(principals))
                .map_err(|_| unreadable(root))?,
            None => book,
        };
        let mut out = BookFiles::new();
        let mut put = |rel: &str, text: String| -> DriveResult<()> {
            let path = root.join(rel)?;
            let _ = out.insert(path, Bytes::from(text));
            Ok(())
        };
        put(
            MANIFEST,
            view.manifest_text().map_err(|_| unreadable(root))?,
        )?;
        put(SUMMARY, view.summary_text())?;
        for (chapter, text) in view.files() {
            put(chapter.as_str(), text.clone())?;
        }
        Ok(out)
    }
}

/// `path` relative to `dir`, when it lies beneath it.
fn relative<'a>(dir: &BlobPath, path: &'a BlobPath) -> Option<&'a str> {
    path.as_str()
        .strip_prefix(dir.as_str())
        .and_then(|rest| rest.strip_prefix('/'))
}

fn unreadable(root: &BlobPath) -> DriveError {
    DriveError::Invalid(format!(
        "the book at {root} could not be read; repair it before copying or opening it"
    ))
}
