// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Path resolution: Drive addresses files by id, Vellum by path.
//!
//! A path is resolved by walking its segments from the root folder with
//! `files.list` (`name = '…' and '<parent>' in parents and trashed =
//! false`). Resolved ids are cached by path for a while. A cached id is
//! never trusted blindly: a cached file id is re-read and must still be
//! untrashed, carry the expected name and sit in the expected folder, and
//! a lookup that comes up empty under a cached folder re-checks that
//! folder. Anything stale is dropped and the walk is redone from the root.

use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

use super::client::{BACKEND, Client, DriveFile, Kind};
use crate::{
    error::{StoreError, StoreResult},
    path::BlobPath,
};

/// Path → Drive id, with the time each entry was learnt.
#[derive(Debug)]
pub(super) struct PathCache {
    ttl: Duration,
    entries: Mutex<HashMap<String, (String, Instant)>>,
}

impl PathCache {
    pub(super) fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, (String, Instant)>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn get(&self, path: &str) -> Option<String> {
        let mut entries = self.lock();
        match entries.get(path) {
            Some((id, at)) if at.elapsed() < self.ttl => Some(id.clone()),
            Some(_) => {
                let _ = entries.remove(path);
                None
            }
            None => None,
        }
    }

    pub(super) fn insert(&self, path: &str, id: &str) {
        if self.ttl.is_zero() {
            return;
        }
        let _ = self
            .lock()
            .insert(path.to_owned(), (id.to_owned(), Instant::now()));
    }

    /// Forgets `path` and everything beneath it.
    pub(super) fn invalidate(&self, path: &str) {
        let nested = format!("{path}/");
        self.lock()
            .retain(|key, _| key != path && !key.starts_with(&nested));
    }

    /// Forgets `path`, everything beneath it, and every folder above it.
    pub(super) fn invalidate_chain(&self, path: &BlobPath) {
        self.invalidate(path.as_str());
        let mut dir = path.parent();
        while let Some(d) = dir {
            let _ = self.lock().remove(d.as_str());
            dir = d.parent();
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

/// A resolved file and the folder it was found in.
#[derive(Debug)]
pub(super) struct Found {
    pub(super) file: DriveFile,
    pub(super) parent: String,
}

/// Picks deterministically among same-named siblings: the oldest
/// `createdTime`, then the smallest id. Drive allows duplicates; every
/// Vellum process makes the same pick, and the others are ignored.
pub(super) fn pick(mut candidates: Vec<DriveFile>, path: &str) -> Option<DriveFile> {
    candidates.sort_by(|a, b| a.age_key().cmp(&b.age_key()));
    if candidates.len() > 1 {
        let ignored: Vec<&str> = candidates.iter().skip(1).map(|f| f.id.as_str()).collect();
        tracing::warn!(
            backend = BACKEND,
            path,
            count = candidates.len(),
            ?ignored,
            "duplicate names in a Drive folder; using the oldest"
        );
    }
    candidates.into_iter().next()
}

/// Walks paths to ids against one root folder.
#[derive(Debug)]
pub(super) struct Resolver {
    pub(super) root: String,
    pub(super) cache: PathCache,
}

impl Resolver {
    /// The id of the folder at `dir` (the root for `None`), and whether a
    /// cached id was relied on. `fresh` ignores the cache (but refills it).
    pub(super) async fn folder(
        &self,
        client: &Client,
        dir: Option<&BlobPath>,
        fresh: bool,
    ) -> StoreResult<(Option<String>, bool)> {
        let Some(dir) = dir else {
            return Ok((Some(self.root.clone()), false));
        };
        // Start from the deepest cached folder.
        let segments: Vec<&str> = dir.segments().collect();
        let mut start = 0;
        let mut current = self.root.clone();
        let mut used_cache = false;
        if !fresh {
            for depth in (1..=segments.len()).rev() {
                let prefix = segments.get(..depth).unwrap_or_default().join("/");
                if let Some(id) = self.cache.get(&prefix) {
                    (start, current, used_cache) = (depth, id, true);
                    break;
                }
            }
        }
        for depth in start..segments.len() {
            let name = segments.get(depth).copied().unwrap_or_default();
            let prefix = segments.get(..=depth).unwrap_or_default().join("/");
            let found = client.find(&current, name, Kind::Folder).await?;
            match pick(found, &prefix) {
                Some(folder) => {
                    self.cache.insert(&prefix, &folder.id);
                    current = folder.id;
                }
                None => return Ok((None, used_cache)),
            }
        }
        Ok((Some(current), used_cache))
    }

    /// The file at `path`, if any (Google-native files included; callers
    /// decide what to do with them).
    pub(super) async fn file(
        &self,
        client: &Client,
        path: &BlobPath,
    ) -> StoreResult<Option<Found>> {
        let parent_path = path.parent();
        let name = path.file_name();
        for fresh in [false, true] {
            let (parent, mut used_cache) = self.folder(client, parent_path.as_ref(), fresh).await?;
            let Some(parent) = parent else {
                if used_cache {
                    self.cache.invalidate_chain(path);
                    continue;
                }
                return Ok(None);
            };
            if !fresh && let Some(id) = self.cache.get(path.as_str()) {
                used_cache = true;
                match client.get_meta(&id, Some(path)).await {
                    Ok(file)
                        if !file.trashed
                            && !file.is_folder()
                            && file.name == name
                            && file.parents.contains(&parent) =>
                    {
                        return Ok(Some(Found { file, parent }));
                    }
                    Ok(_) | Err(StoreError::NotFound(_)) => {
                        tracing::debug!(backend = BACKEND, %path, "cached id is stale");
                        self.cache.invalidate(path.as_str());
                    }
                    Err(e) => return Err(e),
                }
            }
            let found = client.find(&parent, name, Kind::NotFolder).await?;
            if let Some(file) = pick(found, path.as_str()) {
                self.cache.insert(path.as_str(), &file.id);
                return Ok(Some(Found { file, parent }));
            }
            // Nothing here. If the folder came from the cache, make sure it
            // is still live before believing that.
            if used_cache
                && !fresh
                && !self
                    .folder_is_live(client, &parent, parent_path.as_ref())
                    .await?
            {
                self.cache.invalidate_chain(path);
                continue;
            }
            return Ok(None);
        }
        Ok(None)
    }

    async fn folder_is_live(
        &self,
        client: &Client,
        id: &str,
        dir: Option<&BlobPath>,
    ) -> StoreResult<bool> {
        let Some(dir) = dir else {
            return Ok(true); // The root is configured, not resolved.
        };
        match client.get_meta(id, Some(dir)).await {
            Ok(folder) => {
                Ok(!folder.trashed && folder.is_folder() && folder.name == dir.file_name())
            }
            Err(StoreError::NotFound(_)) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: &str, created: &str) -> DriveFile {
        DriveFile {
            id: id.to_owned(),
            created_time: Some(created.to_owned()),
            ..DriveFile::default()
        }
    }

    #[test]
    fn picks_oldest_then_smallest_id() {
        let picked = pick(
            vec![
                file("b", "2026-01-02T00:00:00Z"),
                file("c", "2026-01-01T00:00:00Z"),
                file("a", "2026-01-01T00:00:00.000Z"),
            ],
            "x",
        );
        assert_eq!(picked.map(|f| f.id), Some("a".to_owned()));
    }

    #[test]
    fn invalidation_covers_descendants_and_ancestors() -> StoreResult<()> {
        let cache = PathCache::new(Duration::from_mins(1));
        for (path, id) in [("a", "1"), ("a/b", "2"), ("a/b/c", "3"), ("ab", "4")] {
            cache.insert(path, id);
        }
        cache.invalidate("a/b");
        assert_eq!(cache.get("a").as_deref(), Some("1"));
        assert!(cache.get("a/b/c").is_none());
        assert_eq!(cache.len(), 2);
        cache.invalidate_chain(&BlobPath::new("a/x")?);
        assert!(cache.get("a").is_none());
        assert_eq!(cache.get("ab").as_deref(), Some("4"));
        Ok(())
    }

    #[test]
    fn zero_ttl_disables_the_cache() {
        let cache = PathCache::new(Duration::ZERO);
        cache.insert("a", "1");
        assert!(cache.get("a").is_none());
    }
}
