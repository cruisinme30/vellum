// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Work journal: how several replicas share one set of Work files.
//!
//! Every replica holds the Work engine in memory. To keep the copies in
//! step, each command is numbered in one small blob, `.vellum/work/journal.json`,
//! next to the files it changes. A command:
//!
//! 1. reads the journal and loads the files other replicas changed since
//!    this replica last looked;
//! 2. applies the command in memory;
//! 3. claims the next revision with a conditional write (`If-Match`), so two
//!    replicas can never both commit on the same revision, and the loser
//!    retries on fresh state (no duplicate `ENG-n`);
//! 4. writes its files, then marks its journal entry done.
//!
//! An entry stays `pending` only while its files are written. Writers wait
//! for it; readers stop one revision short. A writer that crashed leaves a
//! pending entry that expires after [`LEASE`], and the files it managed to
//! write are what the others load.
//!
//! The journal keeps the last [`KEEP`] entries. A replica further behind
//! than that rebuilds Work from the files. See ADR 0037 (D150).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use vellum_kernel::{Error, ErrorKind};
use vellum_store::{BlobPath, BlobStore, Etag, Precondition, StoreError};

/// Where the journal lives in the workspace.
pub const PATH: &str = ".vellum/work/journal.json";

/// How many entries the journal keeps.
pub const KEEP: usize = 512;

/// How long a pending entry holds other writers back.
pub const LEASE: Duration = Duration::from_secs(30);

/// One committed (or committing) command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The revision this command produced.
    pub rev: u64,
    /// Every Work file it wrote or removed.
    pub paths: Vec<String>,
    /// Whether its files are still being written.
    #[serde(default)]
    pub pending: bool,
    /// When a pending entry stops holding other writers back (Unix ms).
    #[serde(default)]
    pub expires_ms: u64,
    /// The replica that wrote it, for logs.
    #[serde(default)]
    pub writer: String,
}

/// The journal blob.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    /// The newest revision.
    pub rev: u64,
    /// The newest entries, oldest first.
    pub entries: Vec<Entry>,
}

/// The journal as read, with the etag a claim must match.
#[derive(Debug, Clone)]
pub struct Read {
    /// The content.
    pub journal: Journal,
    /// `None` when no journal exists yet.
    pub etag: Option<Etag>,
}

/// What a replica at `seen` must load to reach the journal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatchUp {
    /// Nothing changed.
    Current,
    /// Load these paths; the replica is then at `rev`.
    Paths {
        /// The paths, deduplicated, in the order they were written.
        paths: Vec<String>,
        /// The revision reached.
        rev: u64,
    },
    /// Too far behind (or the journal was reset): rebuild from the files.
    Reload {
        /// The revision reached.
        rev: u64,
    },
}

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl Journal {
    /// The newest entry, when it is pending and its lease still runs.
    #[must_use]
    pub fn blocking(&self, now_ms: u64) -> Option<&Entry> {
        self.entries
            .last()
            .filter(|e| e.pending && e.expires_ms > now_ms)
    }

    /// The newest revision a reader may load: one short of a live pending
    /// entry, whose files may be half written.
    #[must_use]
    pub fn readable_rev(&self, now_ms: u64) -> u64 {
        self.blocking(now_ms)
            .map_or(self.rev, |e| e.rev.saturating_sub(1))
    }

    /// What a replica that has seen `seen` must do to reach `target`.
    #[must_use]
    pub fn catch_up(&self, seen: u64, target: u64) -> CatchUp {
        if target == seen {
            return CatchUp::Current;
        }
        if target < seen {
            // The journal went backwards: someone removed it. Start over.
            return CatchUp::Reload { rev: target };
        }
        let covered = self.entries.first().is_some_and(|e| e.rev <= seen + 1);
        if !covered {
            return CatchUp::Reload { rev: target };
        }
        let mut paths: Vec<String> = Vec::new();
        for entry in self
            .entries
            .iter()
            .filter(|e| e.rev > seen && e.rev <= target)
        {
            for p in &entry.paths {
                if !paths.contains(p) {
                    paths.push(p.clone());
                }
            }
        }
        CatchUp::Paths { paths, rev: target }
    }

    /// The journal with a pending entry for `paths` appended as `rev + 1`.
    #[must_use]
    pub fn claim(&self, paths: Vec<String>, writer: &str, now_ms: u64) -> Self {
        let mut next = self.clone();
        next.rev += 1;
        next.entries.push(Entry {
            rev: next.rev,
            paths,
            pending: true,
            expires_ms: now_ms.saturating_add(u64::try_from(LEASE.as_millis()).unwrap_or(0)),
            writer: writer.to_owned(),
        });
        let drop = next.entries.len().saturating_sub(KEEP);
        let _ = next.entries.drain(..drop);
        next
    }

    /// The journal with entry `rev` marked done.
    #[must_use]
    pub fn finish(&self, rev: u64) -> Self {
        let mut next = self.clone();
        for e in &mut next.entries {
            if e.rev == rev {
                e.pending = false;
                e.expires_ms = 0;
            }
        }
        next
    }
}

fn path() -> BlobPath {
    BlobPath::new(PATH).unwrap_or_else(|_| unreachable!("the journal path is valid"))
}

fn unavailable(what: &str, e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Unavailable, format!("{what}: {e}"))
}

/// Reads the journal; an absent journal is empty.
///
/// # Errors
///
/// `unavailable` when the store fails or the journal is not valid JSON.
pub async fn read(store: &dyn BlobStore) -> Result<Read, Error> {
    match store.get(&path()).await {
        Ok(blob) => {
            let journal = serde_json::from_slice(&blob.bytes)
                .map_err(|e| unavailable("the Work journal is unreadable", e))?;
            Ok(Read {
                journal,
                etag: Some(blob.meta.etag),
            })
        }
        Err(StoreError::NotFound(_)) => Ok(Read {
            journal: Journal::default(),
            etag: None,
        }),
        Err(e) => Err(unavailable("cannot read the Work journal", e)),
    }
}

/// Writes `journal` if the stored one still has `etag` (or is still absent).
/// Returns the new etag, or `None` when another replica got there first.
///
/// # Errors
///
/// `unavailable` when the store fails.
pub async fn write(
    store: &dyn BlobStore,
    journal: &Journal,
    etag: Option<&Etag>,
) -> Result<Option<Etag>, Error> {
    let bytes = serde_json::to_vec(journal).map_err(|e| unavailable("journal", e))?;
    let condition = etag.map_or(Precondition::IfAbsent, |e| Precondition::IfMatch(e.clone()));
    match store.put(&path(), Bytes::from(bytes), condition).await {
        Ok(meta) => Ok(Some(meta.etag)),
        Err(StoreError::PreconditionFailed { .. } | StoreError::AlreadyExists(_)) => Ok(None),
        Err(e) => Err(unavailable("cannot write the Work journal", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn journal(revs: std::ops::RangeInclusive<u64>) -> Journal {
        let mut j = Journal::default();
        for r in revs {
            j = j.claim(vec![format!("teams/ENG/issues/ENG-{r}.md")], "t", 0);
            j = j.finish(r);
        }
        j
    }

    #[test]
    fn catch_up_lists_each_path_once() {
        let mut j = journal(1..=3);
        j = j
            .claim(vec!["teams/ENG/issues/ENG-2.md".into()], "t", 0)
            .finish(4);
        assert_eq!(j.catch_up(4, 4), CatchUp::Current);
        assert_eq!(
            j.catch_up(1, 4),
            CatchUp::Paths {
                paths: vec![
                    "teams/ENG/issues/ENG-2.md".into(),
                    "teams/ENG/issues/ENG-3.md".into()
                ],
                rev: 4
            }
        );
    }

    #[test]
    fn too_far_behind_or_backwards_reloads() {
        let j = journal(1..=(KEEP as u64 + 10));
        assert_eq!(j.entries.len(), KEEP);
        assert!(matches!(j.catch_up(2, j.rev), CatchUp::Reload { .. }));
        assert!(matches!(
            j.catch_up(j.rev - 5, j.rev),
            CatchUp::Paths { .. }
        ));
        assert!(matches!(
            j.catch_up(j.rev + 5, j.rev),
            CatchUp::Reload { .. }
        ));
    }

    #[test]
    fn a_live_pending_entry_blocks_until_it_expires() {
        let j = journal(1..=2).claim(vec!["x".into()], "w", 1_000);
        assert_eq!(j.rev, 3);
        assert!(j.blocking(1_000).is_some());
        assert_eq!(j.readable_rev(1_000), 2);
        let late = 1_000 + u64::try_from(LEASE.as_millis()).unwrap_or(0) + 1;
        assert!(j.blocking(late).is_none());
        assert_eq!(j.readable_rev(late), 3);
        assert!(j.finish(3).blocking(1_000).is_none());
    }
}
