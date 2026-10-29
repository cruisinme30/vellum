// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Where things live on disk (brief §6: what you see is the disk).
//!
//! ```text
//! .vellum/trash/<id>/manifest.yaml      trash entry; item/ holds the content
//! .vellum/versions/<doc>/<version>.md   version history, with a .yaml sidecar
//! drives/shared/<slug>/                 a shared drive
//! drives/users/<slug>/                  a user's My Drive
//!   .vellum-folder.yaml                 folder (or space) id, icon, settings
//!   Title.md                            a page
//!   Title/                              the pages nested under it (D168)
//!   .vellum-order.yaml                  the order of a container's items (D169)
//!   Name/                               a folder
//!   Name.book/book.yaml                 a book
//!   Name.collection/schema.yaml         a collection
//!   Name.shortcut.yaml                  a shortcut
//! ```

use vellum_kernel::{ObjectId, VersionId};
use vellum_store::BlobPath;

use crate::error::{DriveError, DriveResult};
use crate::model::{NodeKind, SpaceKind};

/// The folder that holds Vellum's own state; never user content.
pub const STATE_DIR: &str = ".vellum";
/// Per-folder (and per-space) metadata file.
pub const FOLDER_FILE: &str = ".vellum-folder.yaml";
/// Per-container sibling order: the child IDs, in order (D169).
pub const ORDER_FILE: &str = ".vellum-order.yaml";
/// The manifest inside a book folder.
pub const BOOK_FILE: &str = "book.yaml";
/// The schema inside a collection folder.
pub const COLLECTION_FILE: &str = "schema.yaml";
/// File name of a trash entry's manifest.
pub const TRASH_MANIFEST: &str = "manifest.yaml";
/// Longest file-name stem, in bytes, leaving room for suffixes.
const MAX_STEM_BYTES: usize = 180;

/// The directory of a space.
///
/// # Errors
/// Fails when `slug` is not a valid single path segment.
pub fn space_root(kind: SpaceKind, slug: &str) -> DriveResult<BlobPath> {
    let group = match kind {
        SpaceKind::SharedDrive => "shared",
        SpaceKind::UserDrive => "users",
    };
    if !is_slug(slug) {
        return Err(DriveError::Invalid(format!(
            "slug {slug:?} must be lowercase letters, digits and dashes"
        )));
    }
    Ok(BlobPath::new(format!("drives/{group}/{slug}"))?)
}

/// Parses a space root path back into its kind and slug.
#[must_use]
pub fn parse_space_root(dir: &BlobPath) -> Option<(SpaceKind, &str)> {
    let mut parts = dir.as_str().split('/');
    let (Some("drives"), Some(group), Some(slug), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let kind = match group {
        "shared" => SpaceKind::SharedDrive,
        "users" => SpaceKind::UserDrive,
        _ => return None,
    };
    is_slug(slug).then_some((kind, slug))
}

/// A My Drive slug from a person's handle: lowercase letters and digits,
/// other characters turned into single dashes, at most 48 bytes, `me`
/// when nothing is left. The web app's `slugOf` does the same.
#[must_use]
pub fn slug_of(handle: &str) -> String {
    let lower = handle.to_lowercase();
    let mut slug = lower
        .split(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit()))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    slug.truncate(48);
    let slug = slug.trim_end_matches('-').to_owned();
    if slug.is_empty() {
        "me".to_owned()
    } else {
        slug
    }
}

fn is_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 64
        && !slug.starts_with('-')
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The directory holding every trash entry.
///
/// # Errors
/// Never fails; the result is propagated for uniformity.
pub fn trash_root() -> DriveResult<BlobPath> {
    Ok(BlobPath::new(format!("{STATE_DIR}/trash"))?)
}

/// The directory of a trash entry.
///
/// # Errors
/// Never fails for a valid ID; the result is propagated for uniformity.
pub fn trash_dir(id: ObjectId) -> DriveResult<BlobPath> {
    Ok(BlobPath::new(format!("{STATE_DIR}/trash/{id}"))?)
}

/// Where a trashed node's files go: `.vellum/trash/<id>/item/<name>`.
///
/// # Errors
/// Fails if `name` is not a valid path segment.
pub fn trash_item(id: ObjectId, name: &str) -> DriveResult<BlobPath> {
    Ok(trash_dir(id)?.join(&format!("item/{name}"))?)
}

/// The directory holding a document's version history.
///
/// # Errors
/// Never fails for a valid ID; the result is propagated for uniformity.
pub fn versions_dir(doc: ObjectId) -> DriveResult<BlobPath> {
    Ok(BlobPath::new(format!("{STATE_DIR}/versions/{doc}"))?)
}

/// The content blob and metadata sidecar of one version.
///
/// # Errors
/// Never fails for valid IDs; the result is propagated for uniformity.
pub fn version_paths(doc: ObjectId, version: VersionId) -> DriveResult<(BlobPath, BlobPath)> {
    let dir = versions_dir(doc)?;
    Ok((
        dir.join(&format!("{version}.md"))?,
        dir.join(&format!("{version}.yaml"))?,
    ))
}

/// True for paths inside Vellum's state folder.
#[must_use]
pub fn is_state(path: &BlobPath) -> bool {
    path.segments().next() == Some(STATE_DIR)
}

/// Turns a title into a safe file-name stem, with the store's naming
/// rules (characters a backend rejects become `-`, leading dots and
/// Windows device names are avoided), leaving room for a suffix.
#[must_use]
pub fn stem(title: &str) -> String {
    let name = vellum_store::layout::file_name(title, "md");
    let mut out = name.strip_suffix(".md").unwrap_or(&name).to_owned();
    while out.len() > MAX_STEM_BYTES {
        let _ = out.pop();
    }
    let out = out.trim_end_matches(['.', ' ']).to_owned();
    if out.is_empty() || out.starts_with(".vellum") {
        return "Untitled".to_owned();
    }
    out
}

/// The file or directory name a node of `kind` takes for `title`, with an
/// optional collision counter: `Title.md`, `Title (2).md`.
#[must_use]
pub fn file_name(kind: NodeKind, title: &str, counter: u32) -> String {
    let base = stem(title);
    let base = if counter > 1 {
        format!("{base} ({counter})")
    } else {
        base
    };
    format!("{base}{}", kind.suffix())
}

/// Recognises a node from a path inside a space, returning its kind and
/// the path of the file that carries its ID.
#[must_use]
pub fn classify(path: &BlobPath) -> Option<(NodeKind, BlobPath)> {
    let name = path.file_name();
    let dir = path.parent();
    if name == FOLDER_FILE {
        return Some((NodeKind::Folder, dir?));
    }
    if let Some(dir) = dir.as_ref() {
        let dir_name = dir.file_name();
        if name == BOOK_FILE && dir_name.ends_with(NodeKind::Book.suffix()) {
            return Some((NodeKind::Book, dir.clone()));
        }
        if name == COLLECTION_FILE && dir_name.ends_with(NodeKind::Collection.suffix()) {
            return Some((NodeKind::Collection, dir.clone()));
        }
    }
    if !vellum_store::layout::Kind::of(path).is_content() {
        return None;
    }
    if name.ends_with(NodeKind::Shortcut.suffix()) {
        return Some((NodeKind::Shortcut, path.clone()));
    }
    if has_ext(name, "md") && !name.starts_with('.') {
        return Some((NodeKind::Page, path.clone()));
    }
    None
}

/// True if a file name has extension `ext`, ignoring case.
#[must_use]
pub fn has_ext(name: &str, ext: &str) -> bool {
    std::path::Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// The file inside a node that carries its ID and title.
///
/// # Errors
/// Fails if the joined path is invalid.
pub fn id_file(kind: NodeKind, path: &BlobPath) -> DriveResult<BlobPath> {
    Ok(match kind {
        NodeKind::Folder => path.join(FOLDER_FILE)?,
        NodeKind::Book => path.join(BOOK_FILE)?,
        NodeKind::Collection => path.join(COLLECTION_FILE)?,
        NodeKind::Page | NodeKind::Shortcut => path.clone(),
    })
}

/// The title a node shows, derived from its path when the file has none.
#[must_use]
pub fn title_from_path(kind: NodeKind, path: &BlobPath) -> String {
    let name = path.file_name();
    name.strip_suffix(kind.suffix()).unwrap_or(name).to_owned()
}

/// Rebases `path` from under `from` to under `to`.
///
/// # Errors
/// Fails if `path` is not under `from` or the result is invalid.
pub fn rebase(path: &BlobPath, from: &BlobPath, to: &BlobPath) -> DriveResult<BlobPath> {
    if path == from {
        return Ok(to.clone());
    }
    let rest = path
        .as_str()
        .strip_prefix(from.as_str())
        .and_then(|r| r.strip_prefix('/'))
        .ok_or_else(|| DriveError::Invalid(format!("{path} is not under {from}")))?;
    Ok(to.join(rest)?)
}

/// The directory that holds the pages nested under the page at `path`:
/// `Plan.md` keeps its children in `Plan/` beside it (D168). `None` when
/// `path` is not a Markdown file.
#[must_use]
pub fn page_dir(path: &BlobPath) -> Option<BlobPath> {
    let name = path.file_name();
    if !has_ext(name, "md") {
        return None;
    }
    let stem = name.get(..name.len().saturating_sub(3))?;
    if stem.is_empty() {
        return None;
    }
    child(path.parent().as_ref(), stem).ok()
}

/// Rebases a member of the node at `from` (the node itself, a file in a
/// folder, book or collection, or a page nested under a page) to the
/// same place under `to`.
///
/// # Errors
/// Fails if `path` is not part of the node or the result is invalid.
pub fn rebase_member(
    kind: NodeKind,
    path: &BlobPath,
    from: &BlobPath,
    to: &BlobPath,
) -> DriveResult<BlobPath> {
    if path.starts_with(from) {
        return rebase(path, from, to);
    }
    if kind == NodeKind::Page
        && let (Some(old), Some(new)) = (page_dir(from), page_dir(to))
        && path.starts_with(&old)
    {
        return rebase(path, &old, &new);
    }
    Err(DriveError::Invalid(format!("{path} is not under {from}")))
}

/// Joins a directory (or the store root for `None`) and a name.
///
/// # Errors
/// Fails if the result is invalid.
pub fn child(dir: Option<&BlobPath>, name: &str) -> DriveResult<BlobPath> {
    Ok(match dir {
        Some(dir) => dir.join(name)?,
        None => BlobPath::new(name)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> BlobPath {
        BlobPath::new(s).unwrap_or_else(|e| unreachable!("{e}"))
    }

    #[test]
    fn stems_are_safe() {
        assert_eq!(stem("Roadmap 2027"), "Roadmap 2027");
        assert_eq!(stem("  a/b:c?  "), "a-b-c-");
        assert_eq!(stem("..hidden."), "hidden");
        assert_eq!(stem("CON"), "CON-untitled");
        assert_eq!(stem(".vellum-folder"), "vellum-folder");
        assert!(stem(&"x".repeat(500)).len() <= MAX_STEM_BYTES);
    }

    #[test]
    fn file_names_carry_kind_and_counter() {
        assert_eq!(file_name(NodeKind::Page, "Plan", 1), "Plan.md");
        assert_eq!(file_name(NodeKind::Page, "Plan", 2), "Plan (2).md");
        assert_eq!(file_name(NodeKind::Book, "Handbook", 1), "Handbook.book");
        assert_eq!(file_name(NodeKind::Folder, "Specs", 1), "Specs");
        assert_eq!(file_name(NodeKind::Shortcut, "Go", 1), "Go.shortcut.yaml");
    }

    #[test]
    fn classifies_paths() {
        let d = "drives/shared/eng";
        assert_eq!(
            classify(&p(&format!("{d}/A.md"))),
            Some((NodeKind::Page, p(&format!("{d}/A.md"))))
        );
        assert_eq!(
            classify(&p(&format!("{d}/F/.vellum-folder.yaml"))),
            Some((NodeKind::Folder, p(&format!("{d}/F"))))
        );
        assert_eq!(
            classify(&p(&format!("{d}/H.book/book.yaml"))),
            Some((NodeKind::Book, p(&format!("{d}/H.book"))))
        );
        assert_eq!(classify(&p(&format!("{d}/F/book.yaml"))), None);
        assert_eq!(classify(&p(&format!("{d}/img.png"))), None);
    }

    #[test]
    fn space_roots_round_trip() {
        let root =
            space_root(SpaceKind::SharedDrive, "eng").unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(root.as_str(), "drives/shared/eng");
        assert_eq!(
            parse_space_root(&root),
            Some((SpaceKind::SharedDrive, "eng"))
        );
        assert!(space_root(SpaceKind::UserDrive, "Bad Slug").is_err());
        assert_eq!(parse_space_root(&p("drives/other/x")), None);
    }

    #[test]
    fn my_drive_slugs_come_from_handles() {
        assert_eq!(slug_of("Ada.Lovelace"), "ada-lovelace");
        assert_eq!(slug_of("__"), "me");
        assert_eq!(slug_of("Zoë_42"), "zo-42");
        assert!(slug_of(&"a".repeat(80)).len() <= 48);
        assert!(is_slug(&slug_of("-x-")));
    }

    #[test]
    fn pages_keep_their_children_beside_them() {
        assert_eq!(page_dir(&p("d/Plan.md")), Some(p("d/Plan")));
        assert_eq!(page_dir(&p("Plan.MD")), Some(p("Plan")));
        assert_eq!(page_dir(&p("d/F")), None);
        let (from, to) = (p("d/Plan.md"), p("e/Goal.md"));
        assert_eq!(
            rebase_member(NodeKind::Page, &p("d/Plan/a/b.md"), &from, &to).ok(),
            Some(p("e/Goal/a/b.md"))
        );
        assert_eq!(
            rebase_member(NodeKind::Page, &from, &from, &to).ok(),
            Some(to.clone())
        );
        assert!(rebase_member(NodeKind::Page, &p("d/Other/x.md"), &from, &to).is_err());
        assert!(rebase_member(NodeKind::Folder, &p("d/Plan/x.md"), &from, &to).is_err());
    }

    #[test]
    fn rebases_paths() {
        assert_eq!(
            rebase(&p("a/b/c.md"), &p("a/b"), &p("x")).ok(),
            Some(p("x/c.md"))
        );
        assert!(rebase(&p("a/bc.md"), &p("a/b"), &p("x")).is_err());
    }
}
