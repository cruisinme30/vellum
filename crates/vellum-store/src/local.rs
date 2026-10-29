// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Local-folder adapter: blobs are plain files under a root directory.
//!
//! Writes go to a hidden temporary file in the target directory, are
//! fsynced, then renamed (or hard-linked for `IfAbsent`) into place, so
//! readers and external editors never see a half-written file.
//! Preconditions are enforced under a per-path lock, which serialises
//! writers inside this process. Editors outside the process cannot take
//! that lock; their changes are caught by the watcher and merged.

use std::{
    fs::{self, File},
    hash::{Hash, Hasher},
    io::{ErrorKind, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::SystemTime,
};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, stream::BoxStream};
use sha2::{Digest, Sha256};
use tracing::{Span, instrument};

use crate::{
    error::{StoreError, StoreResult},
    path::{BlobPath, TEMP_PREFIX},
    store::{Blob, BlobEntry, BlobMeta, BlobStore, Etag, Precondition},
};

const LOCK_STRIPES: u64 = 64;
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A [`BlobStore`] backed by a directory on the local filesystem.
#[derive(Debug, Clone)]
pub struct LocalStore(Arc<Root>);

#[derive(Debug)]
struct Root {
    dir: PathBuf,
    locks: Vec<Mutex<()>>,
}

impl LocalStore {
    /// Opens (creating if needed) a store rooted at `root`.
    ///
    /// # Errors
    /// Returns [`StoreError::Io`] if the directory cannot be created.
    #[instrument(skip_all, fields(root = %root.as_ref().display()), err)]
    pub async fn open(root: impl AsRef<Path>) -> StoreResult<Self> {
        let dir = root.as_ref().to_path_buf();
        let dir = blocking(move || {
            fs::create_dir_all(&dir).at(&dir)?;
            fs::canonicalize(&dir).at(&dir)
        })
        .await?;
        let locks = (0..LOCK_STRIPES).map(|_| Mutex::new(())).collect();
        Ok(Self(Arc::new(Root { dir, locks })))
    }

    async fn run<T, F>(&self, op: F) -> StoreResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Root) -> StoreResult<T> + Send + 'static,
    {
        let root = Arc::clone(&self.0);
        let span = Span::current();
        blocking(move || span.in_scope(|| op(&root))).await
    }
}

#[async_trait]
impl BlobStore for LocalStore {
    fn backend(&self) -> &'static str {
        "local"
    }

    #[instrument(skip(self), fields(backend = "local", path = %path), err)]
    async fn get(&self, path: &BlobPath) -> StoreResult<Blob> {
        let path = path.clone();
        self.run(move |root| {
            let file = root.resolve(&path)?;
            let mut f = open_file(&path, &file)?;
            let mut bytes = Vec::new();
            let _ = f.read_to_end(&mut bytes).at(&file)?;
            let meta = meta(&path, &f, &file, etag_of(&bytes))?;
            let bytes = Bytes::from(bytes);
            Ok(Blob { bytes, meta })
        })
        .await
    }

    #[instrument(skip(self), fields(backend = "local", path = %path), err)]
    async fn head(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        let path = path.clone();
        self.run(move |root| Root::head(&path, &root.resolve(&path)?))
            .await
    }

    #[instrument(skip(self, bytes), fields(backend = "local", path = %path, size = bytes.len()), err)]
    async fn put(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        cond: Precondition,
    ) -> StoreResult<BlobMeta> {
        let path = path.clone();
        self.run(move |root| root.put(&path, &bytes, &cond)).await
    }

    #[instrument(skip(self), fields(backend = "local", path = %path), err)]
    async fn delete(&self, path: &BlobPath, cond: Precondition) -> StoreResult<()> {
        let path = path.clone();
        self.run(move |root| {
            let _guard = root.lock(&[&path]);
            let file = root.resolve(&path)?;
            Root::check(&path, &file, &cond)?;
            fs::remove_file(&file).map_err(|e| not_found_or_io(&path, &file, e))
        })
        .await
    }

    #[instrument(skip(self), fields(backend = "local", from = %from, to = %to), err)]
    async fn rename(&self, from: &BlobPath, to: &BlobPath) -> StoreResult<BlobMeta> {
        let (from, to) = (from.clone(), to.clone());
        self.run(move |root| root.rename(&from, &to)).await
    }

    fn list(&self, dir: Option<&BlobPath>) -> BoxStream<'static, StoreResult<BlobEntry>> {
        let root = Arc::clone(&self.0);
        let start = dir.map_or_else(|| root.dir.clone(), |d| root.dir.join(d.as_str()));
        let span = tracing::info_span!("list", backend = "local", dir = ?dir.map(BlobPath::as_str));
        futures::stream::once(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(256);
            drop(tokio::task::spawn_blocking(move || {
                span.in_scope(|| walk(&root.dir, &start, &tx));
            }));
            futures::stream::unfold(rx, |mut rx| async { rx.recv().await.map(|i| (i, rx)) })
        })
        .flatten()
        .boxed()
    }
}

impl Root {
    /// Maps a blob path to its file, refusing to traverse symlinks so a
    /// link inside the folder can never expose files outside it.
    fn resolve(&self, path: &BlobPath) -> StoreResult<PathBuf> {
        let mut file = self.dir.clone();
        for segment in path.segments() {
            file.push(segment);
            match fs::symlink_metadata(&file) {
                Ok(m) if m.file_type().is_symlink() => {
                    let reason = "traverses a symbolic link";
                    return Err(StoreError::InvalidPath {
                        path: path.to_string(),
                        reason,
                    });
                }
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::NotFound => break,
                Err(e) => return Err(io(&file, e)),
            }
        }
        Ok(self.dir.join(path.as_str()))
    }

    /// Locks the stripes guarding `paths`, in a fixed order to avoid deadlock.
    fn lock(&self, paths: &[&BlobPath]) -> Vec<std::sync::MutexGuard<'_, ()>> {
        let stripes: std::collections::BTreeSet<usize> = paths.iter().map(|p| stripe(p)).collect();
        let locks = stripes.into_iter().filter_map(|i| self.locks.get(i));
        locks
            .map(|m| m.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
            .collect()
    }

    fn head(path: &BlobPath, file: &Path) -> StoreResult<BlobMeta> {
        let mut f = open_file(path, file)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0_u8; 64 * 1024];
        loop {
            match f.read(&mut buf).at(file)? {
                0 => break,
                n => hasher.update(buf.get(..n).unwrap_or_default()),
            }
        }
        meta(path, &f, file, finish(hasher))
    }

    fn check(path: &BlobPath, file: &Path, cond: &Precondition) -> StoreResult<()> {
        let current = match Self::head(path, file) {
            Ok(m) => Some(m.etag),
            Err(StoreError::NotFound(_)) => None,
            Err(e) => return Err(e),
        };
        match cond {
            Precondition::IfAbsent if current.is_some() => {
                Err(StoreError::AlreadyExists(path.clone()))
            }
            Precondition::IfMatch(want) if current.as_ref() != Some(want) => {
                Err(StoreError::PreconditionFailed {
                    path: path.clone(),
                    current,
                })
            }
            _ => Ok(()),
        }
    }

    fn put(&self, path: &BlobPath, bytes: &[u8], cond: &Precondition) -> StoreResult<BlobMeta> {
        let _guard = self.lock(&[path]);
        let file = self.resolve(path)?;
        let dir = file.parent().unwrap_or(&self.dir).to_path_buf();
        fs::create_dir_all(&dir).at(&dir)?;
        Self::check(path, &file, cond)?;
        let tmp = write_temp(&dir, bytes)?;
        let placed = if *cond == Precondition::IfAbsent {
            // A hard link fails atomically if the target appeared meanwhile.
            fs::hard_link(&tmp, &file).map_err(|e| match e.kind() {
                ErrorKind::AlreadyExists => StoreError::AlreadyExists(path.clone()),
                _ => io(&file, e),
            })
        } else {
            fs::rename(&tmp, &file).at(&file)
        };
        let _ = fs::remove_file(&tmp);
        placed?;
        sync_dir(&dir)?;
        meta(path, &open_file(path, &file)?, &file, etag_of(bytes))
    }

    fn rename(&self, from: &BlobPath, to: &BlobPath) -> StoreResult<BlobMeta> {
        let _guard = self.lock(&[from, to]);
        let (src, dst) = (self.resolve(from)?, self.resolve(to)?);
        let dir = dst.parent().unwrap_or(&self.dir).to_path_buf();
        fs::create_dir_all(&dir).at(&dir)?;
        // A hard link fails atomically when `to` exists, so we never clobber.
        match fs::hard_link(&src, &dst) {
            Ok(()) => fs::remove_file(&src).at(&src)?,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                return Err(StoreError::AlreadyExists(to.clone()));
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Err(StoreError::NotFound(from.clone()));
            }
            // Filesystems without hard links (exFAT, some network mounts).
            Err(_) => {
                if fs::exists(&dst).at(&dst)? {
                    return Err(StoreError::AlreadyExists(to.clone()));
                }
                fs::rename(&src, &dst).map_err(|e| not_found_or_io(from, &src, e))?;
            }
        }
        sync_dir(&dir)?;
        Self::head(to, &dst)
    }
}

type Sender = tokio::sync::mpsc::Sender<StoreResult<BlobEntry>>;

/// Depth-first walk that never follows symlinks and hides temp files.
fn walk(root: &Path, start: &Path, tx: &Sender) {
    let mut stack = vec![start.to_path_buf()];
    while let Some(next) = stack.pop() {
        let meta = match fs::symlink_metadata(&next) {
            Ok(m) => m,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return drop(tx.blocking_send(Err(io(&next, e)))),
        };
        if meta.is_dir() {
            match fs::read_dir(&next) {
                Ok(entries) => stack.extend(entries.filter_map(Result::ok).map(|e| e.path())),
                Err(e) => return drop(tx.blocking_send(Err(io(&next, e)))),
            }
        } else if meta.is_file()
            && let Some(path) = to_blob_path(root, &next)
        {
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let entry = BlobEntry {
                path,
                size: meta.len(),
                modified,
            };
            if tx.blocking_send(Ok(entry)).is_err() {
                return; // The consumer dropped the stream.
            }
        }
    }
}

fn to_blob_path(root: &Path, file: &Path) -> Option<BlobPath> {
    let rel = file.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
    let joined = parts?.join("/");
    if joined
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .starts_with(TEMP_PREFIX)
    {
        return None;
    }
    let skip = |e: &StoreError| tracing::warn!(file = %file.display(), error = %e, "skipping file");
    BlobPath::new(joined).inspect_err(skip).ok()
}

fn stripe(path: &BlobPath) -> usize {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    usize::try_from(h.finish() % LOCK_STRIPES).unwrap_or_default()
}

fn write_temp(dir: &Path, bytes: &[u8]) -> StoreResult<PathBuf> {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!("{TEMP_PREFIX}{}-{n}", std::process::id()));
    let written =
        File::create(&tmp).and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written.at(&tmp).map(|()| tmp)
}

/// Makes a completed rename durable across power loss.
fn sync_dir(dir: &Path) -> StoreResult<()> {
    if cfg!(unix) {
        File::open(dir).and_then(|d| d.sync_all()).at(dir)?;
    }
    Ok(())
}

fn open_file(path: &BlobPath, file: &Path) -> StoreResult<File> {
    let f = File::open(file).map_err(|e| not_found_or_io(path, file, e))?;
    let is_file = f.metadata().at(file)?.is_file();
    if is_file {
        Ok(f)
    } else {
        Err(StoreError::NotFound(path.clone()))
    }
}

fn meta(path: &BlobPath, f: &File, file: &Path, etag: Etag) -> StoreResult<BlobMeta> {
    let m = f.metadata().at(file)?;
    let modified = m.modified().at(file)?;
    Ok(BlobMeta {
        path: path.clone(),
        size: m.len(),
        modified,
        etag,
    })
}

fn etag_of(bytes: &[u8]) -> Etag {
    finish(Sha256::new_with_prefix(bytes))
}

fn finish(h: Sha256) -> Etag {
    Etag::new(format!("sha256:{}", hex::encode(h.finalize())))
}

async fn blocking<T: Send + 'static>(
    op: impl FnOnce() -> StoreResult<T> + Send + 'static,
) -> StoreResult<T> {
    let joined = tokio::task::spawn_blocking(op).await;
    joined.map_err(|e| StoreError::Backend(format!("blocking task failed: {e}")))?
}

fn not_found_or_io(path: &BlobPath, file: &Path, e: std::io::Error) -> StoreError {
    if e.kind() == ErrorKind::NotFound {
        StoreError::NotFound(path.clone())
    } else {
        io(file, e)
    }
}

/// Attaches the file path to an I/O error.
trait At<T> {
    fn at(self, file: &Path) -> StoreResult<T>;
}

impl<T> At<T> for std::io::Result<T> {
    fn at(self, file: &Path) -> StoreResult<T> {
        self.map_err(|e| io(file, e))
    }
}

fn io(file: &Path, source: std::io::Error) -> StoreError {
    let path = file.display().to_string();
    StoreError::Io { path, source }
}
