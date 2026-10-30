// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Reading and writing a `name.book/` folder on a local disk.
//!
//! This is the adapter the CLI, the help-book build and the local-folder
//! deployment use; other backends go through the store's `BlobStore` with
//! the same file names. Every path is checked before use: chapter paths
//! cannot leave the book folder (see [`ChapterPath`]) and no file is read
//! or written through a symbolic link. Writes are atomic (temporary file,
//! then rename). Each operation records a span and a
//! `vellum.books.io_ms` histogram.

use std::collections::BTreeSet;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use crate::book::{Book, Repair};
use crate::error::{BookError, Result};
use crate::id::IdSource;
use crate::manifest;
use crate::path::ChapterPath;
use crate::publish::{Site, mdbook_toml};
use crate::snapshot::{self, Snapshot};
use crate::summary;

/// The generated mdBook configuration in a book folder.
pub const MDBOOK_TOML: &str = "book.toml";
/// Where `mdbook build` writes when run on the raw book folder.
pub const MDBOOK_BUILD_DIR: &str = ".mdbook";

fn io_error(path: &Path, source: io::Error) -> BookError {
    BookError::Io {
        path: path.display().to_string(),
        source,
    }
}

/// Joins a relative, `/`-separated path onto `root`, refusing anything
/// that could escape it and any symbolic link on the way.
fn safe_join(root: &Path, relative: &str) -> Result<PathBuf> {
    let unsafe_path = |reason| BookError::InvalidPath {
        path: relative.to_owned(),
        reason,
    };
    let mut out = root.to_path_buf();
    for segment in relative.split('/') {
        let component = Path::new(segment).components().next();
        if segment.is_empty()
            || !matches!(component, Some(Component::Normal(_)))
            || segment.contains('\\')
        {
            return Err(unsafe_path("must be a relative path without '..'"));
        }
        out.push(segment);
        match std::fs::symlink_metadata(&out) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(unsafe_path("symbolic links are not followed"));
            }
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(&out, e)),
        }
    }
    Ok(out)
}

fn read_optional(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_error(path, e)),
    }
}

fn read_required(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).map_err(|e| io_error(path, e))
}

/// Writes `text` atomically, creating parent folders. Returns whether the
/// file changed.
fn write_atomic(path: &Path, text: &str) -> Result<bool> {
    if read_optional(path)?.as_deref() == Some(text) {
        return Ok(false);
    }
    let parent = path.parent().ok_or_else(|| BookError::InvalidPath {
        path: path.display().to_string(),
        reason: "has no parent folder",
    })?;
    std::fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = parent.join(format!(".{name}.vellum-tmp"));
    std::fs::write(&temp, text).map_err(|e| io_error(&temp, e))?;
    std::fs::rename(&temp, path).map_err(|e| io_error(path, e))?;
    Ok(true)
}

fn record(op: &'static str, started: Instant, files: usize) {
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    tracing::info!(
        histogram.vellum.books.io_ms = ms,
        books.op = op,
        books.files = files,
        "book io"
    );
}

/// Loads the book in folder `dir`, repairing drift from outside edits.
///
/// # Errors
///
/// Returns an error when `book.yaml` or `SUMMARY.md` is missing or
/// malformed, a chapter path is unsafe, or a file cannot be read.
pub fn load_dir(
    dir: &Path,
    ids: &mut dyn IdSource,
) -> Result<(Book, Vec<Repair>)> {
    let started = Instant::now();
    let _span =
        tracing::info_span!("books.load", books.dir = %dir.display()).entered();
    let manifest = read_required(&safe_join(dir, manifest::FILE_NAME)?)?;
    let summary = read_required(&safe_join(dir, summary::FILE_NAME)?)?;
    let mut failure: Option<BookError> = None;
    let mut read = |path: &ChapterPath| -> Option<String> {
        match safe_join(dir, path.as_str()).and_then(|p| read_optional(&p)) {
            Ok(text) => text,
            Err(e) => {
                let _ = failure.get_or_insert(e);
                None
            }
        }
    };
    let loaded = Book::load(&manifest, &summary, &mut read, ids)?;
    if let Some(error) = failure {
        return Err(error);
    }
    if !loaded.1.is_empty() {
        tracing::info!(
            books.repairs = loaded.1.len(),
            "repaired outside edits"
        );
    }
    record("load", started, loaded.0.files().len());
    Ok(loaded)
}

/// What [`save_dir`] did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SaveReport {
    /// Files written because their content changed.
    pub written: Vec<String>,
    /// Old locations of moved files that were deleted.
    pub deleted: Vec<String>,
    /// Files of removed sections, left in place for the caller to move to
    /// the drive's trash.
    pub to_trash: Vec<ChapterPath>,
}

/// Writes the book into folder `dir`.
///
/// It writes `book.yaml`, `SUMMARY.md`, a generated `book.toml` (so
/// unmodified `mdbook build` works on the folder) and every chapter whose
/// text changed. Old locations of moved files are deleted; files of
/// removed sections are reported, not deleted.
///
/// # Errors
///
/// Returns an error when a path is unsafe or a file cannot be written.
pub fn save_dir(book: &mut Book, dir: &Path) -> Result<SaveReport> {
    let started = Instant::now();
    let _span = tracing::info_span!(
        "books.save",
        books.dir = %dir.display(),
        book.id = %book.id()
    )
    .entered();
    let mut report = SaveReport::default();
    let write =
        |relative: &str, text: &str, report: &mut SaveReport| -> Result<()> {
            if write_atomic(&safe_join(dir, relative)?, text)? {
                report.written.push(relative.to_owned());
            }
            Ok(())
        };
    for (path, text) in book.files() {
        write(path.as_str(), text, &mut report)?;
    }
    write(manifest::FILE_NAME, &book.manifest_text()?, &mut report)?;
    write(summary::FILE_NAME, &book.summary_text(), &mut report)?;
    write(
        MDBOOK_TOML,
        &mdbook_toml(book.title(), ".", MDBOOK_BUILD_DIR),
        &mut report,
    )?;
    for old in book.moved_away() {
        if book.files().contains_key(old) {
            continue;
        }
        let path = safe_join(dir, old.as_str())?;
        match std::fs::remove_file(&path) {
            Ok(()) => report.deleted.push(old.as_str().to_owned()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(&path, e)),
        }
        remove_empty_parents(dir, &path);
    }
    report.to_trash = book.removed().iter().cloned().collect();
    book.mark_saved();
    record("save", started, report.written.len());
    Ok(report)
}

fn remove_empty_parents(root: &Path, file: &Path) {
    let mut current = file.parent();
    while let Some(dir) = current {
        if dir == root || !dir.starts_with(root) {
            break;
        }
        // Fails (and stops) as soon as a folder is not empty.
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
        current = dir.parent();
    }
}

/// Stores a snapshot in the book folder. Tags are immutable: an existing
/// snapshot file is never overwritten.
///
/// # Errors
///
/// Returns [`BookError::Tag`] when the tag exists, or an I/O error.
pub fn write_snapshot(dir: &Path, snapshot: &Snapshot) -> Result<()> {
    let started = Instant::now();
    let _span =
        tracing::info_span!("books.snapshot.write", books.tag = %snapshot.tag)
            .entered();
    let path = safe_join(dir, &snapshot::file_name(&snapshot.tag)?)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_error(parent, e))?;
    }
    let json = snapshot.to_json()?;
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Err(BookError::Tag {
                tag: snapshot.tag.clone(),
                reason: "already exists; tags are immutable",
            });
        }
        Err(e) => return Err(io_error(&path, e)),
    };
    io::Write::write_all(&mut file, json.as_bytes())
        .map_err(|e| io_error(&path, e))?;
    file.sync_all().map_err(|e| io_error(&path, e))?;
    record("snapshot.write", started, 1);
    Ok(())
}

/// Reads the snapshot tagged `tag` from the book folder.
///
/// # Errors
///
/// Returns an error when the tag is malformed or missing, or the file is
/// not a valid snapshot.
pub fn read_snapshot(dir: &Path, tag: &str) -> Result<Snapshot> {
    let started = Instant::now();
    let _span =
        tracing::info_span!("books.snapshot.read", books.tag = %tag).entered();
    let text = read_required(&safe_join(dir, &snapshot::file_name(tag)?)?)?;
    let snapshot = Snapshot::from_json(&text)?;
    record("snapshot.read", started, 1);
    Ok(snapshot)
}

/// The tags of every snapshot in the book folder.
///
/// # Errors
///
/// Returns an I/O error when the folder cannot be listed.
pub fn list_tags(dir: &Path) -> Result<BTreeSet<String>> {
    let path = safe_join(dir, snapshot::DIR)?;
    let entries = match std::fs::read_dir(&path) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(BTreeSet::new());
        }
        Err(e) => return Err(io_error(&path, e)),
    };
    let mut tags = BTreeSet::new();
    for entry in entries {
        let entry = entry.map_err(|e| io_error(&path, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(tag) = name.strip_suffix(".json")
            && snapshot::validate_tag(tag).is_ok()
        {
            let _ = tags.insert(tag.to_owned());
        }
    }
    Ok(tags)
}

/// Writes a published site or mdBook tree into folder `out`.
///
/// # Errors
///
/// Returns an error when a path is unsafe or a file cannot be written.
pub fn write_site(out: &Path, site: &Site) -> Result<()> {
    let started = Instant::now();
    let _span =
        tracing::info_span!("books.site.write", books.out = %out.display())
            .entered();
    for (relative, text) in &site.files {
        let _ = write_atomic(&safe_join(out, relative)?, text)?;
    }
    record("site.write", started, site.files.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::book::fixtures::{handbook, id};
    use crate::id::SequentialIds;
    use crate::ops::Place;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "vellum-books-{}-{name}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn save_load_round_trip_and_moves() {
        let tmp = TempDir::new("roundtrip");
        let dir = tmp.0.join("handbook.book");
        let mut book = handbook();
        let report = save_dir(&mut book, &dir).unwrap();
        assert!(
            report
                .written
                .contains(&"architecture/services.md".to_owned())
        );
        assert!(dir.join("book.toml").exists());
        let (loaded, repairs) =
            load_dir(&dir, &mut SequentialIds::default()).unwrap();
        assert!(repairs.is_empty(), "{repairs:?}");
        assert_eq!(loaded, book);

        // Un-nest services: architecture/ empties and goes away.
        assert!(
            book.move_section(
                &id(4).into(),
                &Place::Start(crate::summary::Section::Suffix)
            )
            .is_err()
        );
        book.move_section(&id(5).into(), &Place::After(id(4).into()))
            .unwrap();
        let report = save_dir(&mut book, &dir).unwrap();
        assert!(
            report
                .deleted
                .contains(&"architecture/services.md".to_owned())
        );
        assert!(!dir.join("architecture").exists());
        assert!(dir.join("services.md").exists());
        // A second save writes nothing.
        assert!(save_dir(&mut book, &dir).unwrap().written.is_empty());
    }

    #[test]
    fn snapshots_on_disk() {
        let tmp = TempDir::new("snapshots");
        let book = handbook();
        let snap = book
            .snapshot(
                "v1.0",
                "First",
                "2026-10-29T08:00:00Z",
                "user:maya",
                &list_tags(&tmp.0).unwrap(),
            )
            .unwrap();
        write_snapshot(&tmp.0, &snap).unwrap();
        assert!(write_snapshot(&tmp.0, &snap).is_err());
        assert_eq!(
            list_tags(&tmp.0).unwrap(),
            BTreeSet::from(["v1.0".to_owned()])
        );
        assert_eq!(read_snapshot(&tmp.0, "v1.0").unwrap(), snap);
        assert!(read_snapshot(&tmp.0, "../escape").is_err());
    }

    #[test]
    fn refuses_unsafe_paths() {
        let tmp = TempDir::new("unsafe");
        assert!(safe_join(&tmp.0, "../x").is_err());
        assert!(safe_join(&tmp.0, "/etc/passwd").is_err());
        assert!(safe_join(&tmp.0, "a//b").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp", tmp.0.join("link")).unwrap();
            assert!(safe_join(&tmp.0, "link/x.md").is_err());
        }
    }
}
