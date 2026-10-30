// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Google Drive adapter (feature `gdrive`): blobs are files beneath one
//! folder in My Drive or a shared drive, reached through Drive API v3.
//!
//! # Consistency: best effort, always merged
//!
//! Drive has no conditional writes: `files.update` takes no `If-Match`.
//! This adapter therefore checks every [`Precondition`] just before it
//! writes, which is *best effort*: two writers racing inside that window
//! can both land, and the later one becomes the head revision. That is
//! acceptable for Drive because Vellum treats cloud-drive folders as
//! eventually consistent and always merged, never last-write-wins: Drive
//! keeps the overwritten revision, and Vellum's watcher notices the
//! change and reconciles the two versions with its three-way merge. The
//! adapter runs the conformance suite with
//! `Guarantees::BestEffort`, which skips only the racing-writers check;
//! sequential precondition checks hold exactly as on atomic backends.
//!
//! Two small steps narrow the window where Drive allows it: an
//! [`Precondition::IfAbsent`] create re-reads the folder afterwards and,
//! if an older file of the same name appeared meanwhile, deletes its own
//! copy and reports [`StoreError::AlreadyExists`]; and a newly created
//! folder that turns out to have an older twin is removed in favour of
//! the twin. Concurrent folder creation inside one process is serialised.
//!
//! # Versions
//!
//! A blob's [`Etag`] is the file's `headRevisionId` (see
//! `DriveFile::etag`): it changes on every content upload and on nothing
//! else. `get` reads metadata before the bytes, so if the file changes in
//! between, the bytes are newer than the etag, never older: the caller's
//! next conditional write then fails and merges instead of overwriting.
//!
//! # Paths, ids and duplicates
//!
//! Drive addresses files by id and allows several files of the same name
//! in one folder. Paths are resolved segment by segment with `files.list`
//! and cached (see [`GoogleDriveConfig::cache_ttl`]); cached ids are
//! re-validated before use and dropped on rename, delete and `NotFound`.
//! Among same-named siblings the adapter always picks the oldest
//! (`createdTime`, then id), logs a warning, and ignores the rest, in
//! resolution and listings alike, so every Vellum process sees the same
//! file. A folder and a file may share a name; paths through it use the
//! folder, and the blob at it is the file. Names containing `/` cannot be
//! expressed as a [`BlobPath`] and are skipped.
//!
//! # Google-native files and shortcuts
//!
//! Docs, Sheets, Slides, shortcuts and other `application/vnd.google-apps.*`
//! files have no bytes. Listings skip them, and every other operation on
//! one fails with [`StoreError::Backend`] rather than touching it.
//!
//! # Deletes
//!
//! `delete` removes the file permanently with `files.delete` by default.
//! Vellum keeps its own trash in `.vellum/trash`, so moving the file to
//! Drive's trash as well would keep a second copy that counts against the
//! owner's quota and confuses restores. On a shared drive, permanent
//! deletion needs the organizer (Manager) role; set
//! [`GoogleDriveConfig::trash_on_delete`] when Vellum only has the
//! content-manager role there.
//!
//! # Errors and retries
//!
//! `404` is [`StoreError::NotFound`]. A `401` drops the cached access
//! token ([`TokenSource::invalidate`]) and retries once, then fails with
//! [`StoreError::Backend`]. `429` and `5xx` are retried by
//! [`crate::remote::Http`]; Drive's `403 rateLimitExceeded` and
//! `userRateLimitExceeded` are retried here with a bounded backoff. Tokens
//! and resumable-session URIs are never logged.

mod client;
mod resolve;
mod time;

use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, stream::BoxStream};
use tracing::Instrument;

use self::{
    client::{BACKEND, Client, DriveFile, Kind},
    resolve::{Found, PathCache, Resolver, pick},
};
use super::TokenSource;
use crate::{
    error::{StoreError, StoreResult},
    path::{BlobPath, TEMP_PREFIX},
    store::{Blob, BlobEntry, BlobMeta, BlobStore, Etag, Precondition},
};

/// Drive API v3 base URL.
pub const DEFAULT_API_BASE: &str = "https://www.googleapis.com/drive/v3";
/// Drive API v3 upload base URL.
pub const DEFAULT_UPLOAD_BASE: &str =
    "https://www.googleapis.com/upload/drive/v3";
/// Uploads larger than this use a resumable session (5 `MiB`).
pub const DEFAULT_RESUMABLE_THRESHOLD: usize = 5 * 1024 * 1024;
/// Size of each resumable chunk (8 `MiB`, a multiple of 256 `KiB`).
pub const DEFAULT_RESUMABLE_CHUNK: usize = 8 * 1024 * 1024;
/// How long a resolved path → id mapping is reused (5 minutes).
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_mins(5);

/// Configuration for a [`GoogleDriveStore`]. Build it with
/// [`GoogleDriveConfig::new`] and adjust the public fields.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GoogleDriveConfig {
    /// Drive API base URL, [`DEFAULT_API_BASE`] unless testing.
    pub api_base: String,
    /// Upload base URL, [`DEFAULT_UPLOAD_BASE`] unless testing.
    pub upload_base: String,
    /// Id of the folder holding the store: a folder in My Drive, or a
    /// shared drive (or a folder in one).
    pub root_folder_id: String,
    /// The shared drive's id when the root lies in a shared drive. Listings
    /// then search that drive (`corpora=drive`) instead of the user's
    /// files.
    pub drive_id: Option<String>,
    /// Bearer tokens for the API.
    pub tokens: Arc<dyn TokenSource>,
    /// Uploads larger than this many bytes use a resumable session.
    pub resumable_threshold: usize,
    /// Bytes per resumable request, rounded down to a multiple of 256 `KiB`.
    pub resumable_chunk: usize,
    /// `pageSize` for `files.list` (Drive caps it at 1000).
    pub page_size: u32,
    /// How long a resolved path → id mapping is reused before it is looked
    /// up again; zero disables the cache.
    pub cache_ttl: Duration,
    /// Move deleted files to Drive's trash instead of deleting them
    /// permanently.
    pub trash_on_delete: bool,
}

impl GoogleDriveConfig {
    /// A configuration for the folder `root_folder_id`, with defaults for
    /// everything else.
    #[must_use]
    pub fn new(
        root_folder_id: impl Into<String>,
        tokens: Arc<dyn TokenSource>,
    ) -> Self {
        Self {
            api_base: DEFAULT_API_BASE.to_owned(),
            upload_base: DEFAULT_UPLOAD_BASE.to_owned(),
            root_folder_id: root_folder_id.into(),
            drive_id: None,
            tokens,
            resumable_threshold: DEFAULT_RESUMABLE_THRESHOLD,
            resumable_chunk: DEFAULT_RESUMABLE_CHUNK,
            page_size: 1000,
            cache_ttl: DEFAULT_CACHE_TTL,
            trash_on_delete: false,
        }
    }
}

/// A [`BlobStore`] backed by a Google Drive folder. See the module docs for
/// its consistency model.
#[derive(Clone, Debug)]
pub struct GoogleDriveStore(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    client: Client,
    resolver: Resolver,
    /// Serialises folder creation so this process never makes twins.
    folders: tokio::sync::Mutex<()>,
    trash_on_delete: bool,
}

impl GoogleDriveStore {
    /// A store over the folder `config.root_folder_id`. Nothing is sent
    /// until the first operation.
    ///
    /// # Errors
    /// Returns [`StoreError::Backend`] if the HTTP client cannot start.
    pub fn new(config: GoogleDriveConfig) -> StoreResult<Self> {
        let client = Client::new(
            &config.api_base,
            &config.upload_base,
            config.tokens,
            config.drive_id,
            config.page_size,
            config.resumable_threshold,
            config.resumable_chunk,
        )?;
        Ok(Self(Arc::new(Inner {
            client,
            resolver: Resolver {
                root: config.root_folder_id,
                cache: PathCache::new(config.cache_ttl),
            },
            folders: tokio::sync::Mutex::new(()),
            trash_on_delete: config.trash_on_delete,
        })))
    }
}

fn native(path: &BlobPath, file: &DriveFile) -> StoreError {
    StoreError::Backend(format!(
        "{BACKEND}: {path} is a Google-native file ({}) with no bytes; \
         Vellum cannot read or write it",
        file.mime_type
    ))
}

/// Applies `cond` to what is stored now, the way atomic backends do:
/// `IfAbsent` fails on any file (native ones included), and `IfMatch`
/// fails with the current etag, or `None` when nothing is there.
fn check(
    path: &BlobPath,
    found: Option<&Found>,
    cond: &Precondition,
) -> StoreResult<()> {
    if *cond == Precondition::IfAbsent && found.is_some() {
        return Err(StoreError::AlreadyExists(path.clone()));
    }
    if let Some(found) = found
        && found.file.is_native()
    {
        return Err(native(path, &found.file));
    }
    if let Precondition::IfMatch(want) = cond {
        let current: Option<Etag> =
            found.map(|f| f.file.etag(path)).transpose()?;
        if current.as_ref() != Some(want) {
            return Err(StoreError::PreconditionFailed {
                path: path.clone(),
                current,
            });
        }
    }
    Ok(())
}

impl Inner {
    /// The folder at `dir`, creating any missing folders on the way.
    async fn ensure_dirs(&self, dir: Option<&BlobPath>) -> StoreResult<String> {
        if let (Some(id), _) =
            self.resolver.folder(&self.client, dir, false).await?
        {
            return Ok(id);
        }
        let Some(dir) = dir else {
            return Ok(self.resolver.root.clone());
        };
        let _creating = self.folders.lock().await;
        let mut current = self.resolver.root.clone();
        let mut prefix = String::new();
        for name in dir.segments() {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(name);
            if let Some(id) = self.resolver.cache.get(&prefix) {
                current = id;
                continue;
            }
            let existing =
                self.client.find(&current, name, Kind::Folder).await?;
            let folder = match pick(existing, &prefix) {
                Some(folder) => folder,
                None => self.create_folder(&current, name, &prefix).await?,
            };
            self.resolver.cache.insert(&prefix, &folder.id);
            current = folder.id;
        }
        Ok(current)
    }

    /// Creates a folder, then defers to an older twin if another writer
    /// created one meanwhile, removing ours while it is still empty.
    async fn create_folder(
        &self,
        parent: &str,
        name: &str,
        prefix: &str,
    ) -> StoreResult<DriveFile> {
        let created = self.client.create_folder(parent, name).await?;
        tracing::debug!(backend = BACKEND, folder = prefix, "created folder");
        let twins = self.client.find(parent, name, Kind::Folder).await?;
        match pick(twins, prefix) {
            Some(oldest) if oldest.id != created.id => {
                if let Err(e) =
                    self.client.remove(&created.id, None, false).await
                {
                    tracing::warn!(
                        backend = BACKEND,
                        folder = prefix,
                        error = %e,
                        "cannot remove duplicate folder"
                    );
                }
                Ok(oldest)
            }
            _ => Ok(created),
        }
    }

    /// Drops the cached id after the backend said `NotFound` for it.
    fn forget_on_not_found<T>(
        &self,
        path: &BlobPath,
        result: StoreResult<T>,
    ) -> StoreResult<T> {
        if matches!(result, Err(StoreError::NotFound(_))) {
            self.resolver.cache.invalidate_chain(path);
        }
        result
    }

    async fn resolve(&self, path: &BlobPath) -> StoreResult<Option<Found>> {
        self.resolver.file(&self.client, path).await
    }

    async fn put(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        cond: &Precondition,
    ) -> StoreResult<BlobMeta> {
        let found = self.resolve(path).await?;
        check(path, found.as_ref(), cond)?;
        if let Some(found) = found {
            let updated = self
                .client
                .update_content(&found.file.id, path, bytes.clone())
                .await;
            match self.forget_on_not_found(path, updated) {
                Ok(file) => return file.meta(path),
                // Deleted since the check: an `IfMatch` writer has lost,
                // anyone else creates the file afresh.
                Err(StoreError::NotFound(_))
                    if matches!(cond, Precondition::IfMatch(_)) =>
                {
                    return Err(StoreError::PreconditionFailed {
                        path: path.clone(),
                        current: None,
                    });
                }
                Err(StoreError::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        let parent_path = path.parent();
        let mut parent = self.ensure_dirs(parent_path.as_ref()).await?;
        let mut created =
            self.client.create_file(&parent, path, bytes.clone()).await;
        if matches!(created, Err(StoreError::NotFound(_))) {
            // A cached folder vanished under us: walk again and retry once.
            self.resolver.cache.invalidate_chain(path);
            parent = self.ensure_dirs(parent_path.as_ref()).await?;
            created = self.client.create_file(&parent, path, bytes).await;
        }
        let file = created?;
        if *cond == Precondition::IfAbsent {
            let twins = self
                .client
                .find(&parent, path.file_name(), Kind::NotFolder)
                .await?;
            if let Some(oldest) = pick(twins, path.as_str())
                && oldest.id != file.id
            {
                // Another writer created it first: withdraw ours.
                self.client.remove(&file.id, Some(path), false).await?;
                self.resolver.cache.insert(path.as_str(), &oldest.id);
                return Err(StoreError::AlreadyExists(path.clone()));
            }
        }
        self.resolver.cache.insert(path.as_str(), &file.id);
        file.meta(path)
    }

    async fn delete(
        &self,
        path: &BlobPath,
        cond: &Precondition,
    ) -> StoreResult<()> {
        let found = self.resolve(path).await?;
        check(path, found.as_ref(), cond)?;
        let found = found.ok_or_else(|| StoreError::NotFound(path.clone()))?;
        let removed = self
            .client
            .remove(&found.file.id, Some(path), self.trash_on_delete)
            .await;
        self.resolver.cache.invalidate(path.as_str());
        self.forget_on_not_found(path, removed)
    }

    async fn rename(
        &self,
        from: &BlobPath,
        to: &BlobPath,
    ) -> StoreResult<BlobMeta> {
        let src = self
            .resolve(from)
            .await?
            .ok_or_else(|| StoreError::NotFound(from.clone()))?;
        if src.file.is_native() {
            return Err(native(from, &src.file));
        }
        if self.resolve(to).await?.is_some() {
            return Err(StoreError::AlreadyExists(to.clone()));
        }
        let parent = self.ensure_dirs(to.parent().as_ref()).await?;
        let (add, remove) = if parent == src.parent {
            (None, None)
        } else {
            (Some(parent.as_str()), Some(src.parent.as_str()))
        };
        let moved = self
            .client
            .move_file(&src.file.id, from, to.file_name(), add, remove)
            .await;
        self.resolver.cache.invalidate(from.as_str());
        let moved = self.forget_on_not_found(from, moved)?;
        self.resolver.cache.insert(to.as_str(), &moved.id);
        moved.meta(to)
    }

    /// The blobs directly in folder `id` (at `prefix`) and its subfolders.
    async fn children(
        &self,
        id: &str,
        prefix: Option<&BlobPath>,
    ) -> StoreResult<(Vec<BlobEntry>, Vec<(String, BlobPath)>)> {
        let mut all = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self
                .client
                .list_page(id, None, Kind::Any, token.as_deref())
                .await?;
            all.extend(page.files.into_iter().filter(|f| !f.trashed));
            match page.next_page_token {
                Some(next) if !next.is_empty() => token = Some(next),
                _ => break,
            }
        }
        // Group same-named siblings so duplicates resolve as `get` would.
        let mut folders: BTreeMap<String, Vec<DriveFile>> = BTreeMap::new();
        let mut files: BTreeMap<String, Vec<DriveFile>> = BTreeMap::new();
        for f in all {
            let group = if f.is_folder() {
                &mut folders
            } else {
                &mut files
            };
            group.entry(f.name.clone()).or_default().push(f);
        }
        let mut entries = Vec::new();
        for (name, same) in files {
            let Some(path) = child_path(prefix, &name) else {
                continue;
            };
            match pick(same, path.as_str()) {
                Some(f) if f.is_native() => {
                    tracing::debug!(
                        backend = BACKEND,
                        %path,
                        mime = %f.mime_type,
                        "skipping Google-native file"
                    );
                }
                Some(f) => entries.push(BlobEntry {
                    path,
                    size: f.size(),
                    modified: f.modified(),
                }),
                None => {}
            }
        }
        let mut subfolders = Vec::new();
        for (name, same) in folders {
            let Some(path) = child_path(prefix, &name) else {
                continue;
            };
            if let Some(f) = pick(same, path.as_str()) {
                subfolders.push((f.id, path));
            }
        }
        Ok((entries, subfolders))
    }

    /// Where a listing starts: the folder at `dir`, or the file there.
    async fn list_start(
        &self,
        dir: Option<&BlobPath>,
    ) -> StoreResult<ListStart> {
        if let (Some(id), _) =
            self.resolver.folder(&self.client, dir, false).await?
        {
            return Ok(ListStart::Folder(id));
        }
        // A stale cached folder would hide a live one: retry uncached.
        if let (Some(id), _) =
            self.resolver.folder(&self.client, dir, true).await?
        {
            return Ok(ListStart::Folder(id));
        }
        let Some(dir) = dir else {
            return Ok(ListStart::Nothing);
        };
        Ok(match self.resolve(dir).await? {
            Some(found) if !found.file.is_native() => {
                ListStart::File(BlobEntry {
                    path: dir.clone(),
                    size: found.file.size(),
                    modified: found.file.modified(),
                })
            }
            _ => ListStart::Nothing,
        })
    }
}

enum ListStart {
    Folder(String),
    File(BlobEntry),
    Nothing,
}

/// The path of child `name` under `prefix`, or `None` for names Vellum
/// hides (temporary files) or cannot express.
fn child_path(prefix: Option<&BlobPath>, name: &str) -> Option<BlobPath> {
    if name.starts_with(TEMP_PREFIX) {
        return None;
    }
    if name.contains('/') {
        tracing::warn!(
            backend = BACKEND,
            name,
            "skipping a Drive name containing '/'"
        );
        return None;
    }
    let joined = prefix.map_or_else(|| BlobPath::new(name), |p| p.join(name));
    joined
        .inspect_err(|e| {
            tracing::warn!(
                backend = BACKEND,
                name,
                error = %e,
                "skipping file"
            );
        })
        .ok()
}

/// The directory a listing was asked for (`None` for the whole store).
struct Start(Option<BlobPath>);

/// Breadth-first walk state for [`BlobStore::list`].
struct Walk {
    inner: Arc<Inner>,
    /// The directory to start from, until the walk has started.
    start: Option<Start>,
    queue: VecDeque<(String, Option<BlobPath>)>,
    seen: HashSet<String>,
    done: bool,
}

impl Walk {
    /// The next batch of entries, or `None` when the walk is over.
    async fn step(&mut self) -> Option<Vec<StoreResult<BlobEntry>>> {
        if self.done {
            return None;
        }
        if let Some(Start(dir)) = self.start.take() {
            return Some(match self.inner.list_start(dir.as_ref()).await {
                Ok(ListStart::Folder(id)) => {
                    self.queue.push_back((id, dir));
                    Vec::new()
                }
                Ok(ListStart::File(entry)) => {
                    self.done = true;
                    vec![Ok(entry)]
                }
                Ok(ListStart::Nothing) => {
                    self.done = true;
                    Vec::new()
                }
                Err(e) => {
                    self.done = true;
                    vec![Err(e)]
                }
            });
        }
        let (id, prefix) = self.queue.pop_front()?;
        if !self.seen.insert(id.clone()) {
            return Some(Vec::new()); // Reached twice through multiple parents.
        }
        Some(match self.inner.children(&id, prefix.as_ref()).await {
            Ok((entries, subfolders)) => {
                self.queue.extend(
                    subfolders
                        .into_iter()
                        .map(|(folder, path)| (folder, Some(path))),
                );
                entries.into_iter().map(Ok).collect()
            }
            Err(e) => {
                self.done = true;
                vec![Err(e)]
            }
        })
    }
}

#[async_trait]
impl BlobStore for GoogleDriveStore {
    fn backend(&self) -> &'static str {
        BACKEND
    }

    #[tracing::instrument(
        skip(self),
        fields(backend = "gdrive", path = %path),
        err
    )]
    async fn get(&self, path: &BlobPath) -> StoreResult<Blob> {
        let inner = &self.0;
        let found = inner
            .resolve(path)
            .await?
            .ok_or_else(|| StoreError::NotFound(path.clone()))?;
        if found.file.is_native() {
            return Err(native(path, &found.file));
        }
        // Metadata first: if the file changes before the download, the
        // bytes are newer than the etag, so a stale write fails safely.
        let mut meta = found.file.meta(path)?;
        let downloaded = inner.client.download(&found.file.id, path).await;
        let bytes = inner.forget_on_not_found(path, downloaded)?;
        tracing::debug!(
            md5 = found.file.md5_checksum.as_deref(),
            size = bytes.len(),
            "downloaded"
        );
        meta.size = bytes.len() as u64;
        Ok(Blob { bytes, meta })
    }

    #[tracing::instrument(
        skip(self),
        fields(backend = "gdrive", path = %path),
        err
    )]
    async fn head(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        let found = self
            .0
            .resolve(path)
            .await?
            .ok_or_else(|| StoreError::NotFound(path.clone()))?;
        if found.file.is_native() {
            return Err(native(path, &found.file));
        }
        found.file.meta(path)
    }

    #[tracing::instrument(
        skip(self, bytes),
        fields(backend = "gdrive", path = %path, size = bytes.len()),
        err
    )]
    async fn put(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        condition: Precondition,
    ) -> StoreResult<BlobMeta> {
        self.0.put(path, bytes, &condition).await
    }

    #[tracing::instrument(
        skip(self),
        fields(backend = "gdrive", path = %path),
        err
    )]
    async fn delete(
        &self,
        path: &BlobPath,
        condition: Precondition,
    ) -> StoreResult<()> {
        self.0.delete(path, &condition).await
    }

    #[tracing::instrument(
        skip(self),
        fields(backend = "gdrive", from = %from, to = %to),
        err
    )]
    async fn rename(
        &self,
        from: &BlobPath,
        to: &BlobPath,
    ) -> StoreResult<BlobMeta> {
        self.0.rename(from, to).await
    }

    fn list(
        &self,
        dir: Option<&BlobPath>,
    ) -> BoxStream<'static, StoreResult<BlobEntry>> {
        let span = tracing::info_span!(
            "list",
            backend = "gdrive",
            dir = ?dir.map(BlobPath::as_str)
        );
        let walk = Walk {
            inner: Arc::clone(&self.0),
            start: Some(Start(dir.cloned())),
            queue: VecDeque::new(),
            seen: HashSet::new(),
            done: false,
        };
        futures::stream::unfold(walk, move |mut walk| {
            let span = span.clone();
            async move {
                walk.step()
                    .instrument(span)
                    .await
                    .map(|batch| (batch, walk))
            }
        })
        .flat_map(futures::stream::iter)
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_paths_hide_temp_and_slashed_names() -> StoreResult<()> {
        let dir = BlobPath::new("notes")?;
        assert_eq!(
            child_path(Some(&dir), "a.md"),
            Some(BlobPath::new("notes/a.md")?)
        );
        assert_eq!(child_path(None, "a.md"), Some(BlobPath::new("a.md")?));
        assert_eq!(child_path(Some(&dir), &format!("{TEMP_PREFIX}1")), None);
        assert_eq!(child_path(Some(&dir), "a/b"), None);
        assert_eq!(child_path(Some(&dir), ".."), None);
        Ok(())
    }
}
