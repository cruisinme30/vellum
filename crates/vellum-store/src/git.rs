// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Git as a first-class backend.
//!
//! The drive is an ordinary Git work tree. Reads and writes go straight to
//! the files through [`LocalStore`], so the editor, the watcher and the
//! conformance suite see a normal folder. On top of that:
//!
//! - [`GitStore::commit`] records one commit per save batch, covering
//!   everything that changed in the work tree, including edits made with
//!   other tools.
//! - [`GitStore::sync`] fetches the remote branch and merges it. Files that
//!   both sides changed go through the same three-way merge as concurrent
//!   saves: clean merges are committed, and real conflicts keep the
//!   remote's version at the path and the local version in a conflict
//!   copy. Then it pushes. History is never rewritten and never
//!   force-pushed, so other clones stay valid.
//!
//! The `git` command-line tool does the repository work. It must be
//! installed, and pushing uses whatever credentials Git is configured
//! with (a credential helper, an SSH key or a token in the remote URL).
//! Repository hooks are not run.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, stream::BoxStream};
use tokio::{io::AsyncWriteExt, process::Command, sync::RwLock};

use crate::{
    error::{StoreError, StoreResult},
    layout::SYSTEM_DIR,
    local::LocalStore,
    merge::MergeKind,
    path::BlobPath,
    reconcile::{Writer, conflict_path, three_way},
    store::{Blob, BlobEntry, BlobMeta, BlobStore, Precondition},
};

/// Directory Git keeps its own data in; never content.
const GIT_DIR: &str = ".git";
/// Fetch, merge and push rounds before giving up on a busy remote.
const MAX_ROUNDS: usize = 5;
/// Conflict copies tried per path before giving up.
const MAX_COPIES: usize = 8;

/// Where and as whom a [`GitStore`] commits and syncs.
#[derive(Clone, Debug)]
pub struct GitConfig {
    /// Remote to sync with, such as `origin`. Sync only commits when the
    /// repository has no remote of this name.
    pub remote: String,
    /// Branch to commit to and sync, such as `main`.
    pub branch: String,
    /// Name recorded as author and committer.
    pub author_name: String,
    /// Email recorded as author and committer.
    pub author_email: String,
}

/// What [`GitStore::sync`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Synced {
    /// The local save batch committed before syncing, if anything changed.
    pub committed: Option<String>,
    /// Paths the remote changed in the work tree, for the indexer.
    pub updated: Vec<BlobPath>,
    /// Paths both sides changed that merged cleanly.
    pub merged: Vec<BlobPath>,
    /// Conflicts, as `(path, copy)`: `path` keeps the remote's version
    /// and `copy` holds the local one.
    pub conflicts: Vec<(BlobPath, BlobPath)>,
    /// Whether the branch was pushed.
    pub pushed: bool,
}

/// A [`BlobStore`] over a Git work tree, with commit and sync.
#[derive(Debug)]
pub struct GitStore {
    local: LocalStore,
    dir: PathBuf,
    config: GitConfig,
    /// Writes share it and sync takes it alone, so a merge never races a
    /// save.
    gate: RwLock<()>,
}

impl GitStore {
    /// Opens the Git work tree at `dir`, creating the folder and running
    /// `git init` if it is not a repository yet. Vellum's own state
    /// (`.vellum/`) is excluded from commits.
    ///
    /// # Errors
    /// Returns [`StoreError::Backend`] if `git` is missing or fails, or I/O
    /// errors from creating the folder.
    #[tracing::instrument(skip_all, fields(dir = %dir.as_ref().display()), err)]
    pub async fn open(dir: impl AsRef<Path>, config: GitConfig) -> StoreResult<Self> {
        let local = LocalStore::open(dir.as_ref()).await?;
        let dir = dir
            .as_ref()
            .canonicalize()
            .map_err(|source| StoreError::Io {
                path: dir.as_ref().display().to_string(),
                source,
            })?;
        let this = Self {
            local,
            dir,
            config,
            gate: RwLock::new(()),
        };
        let top = this.git(&["rev-parse", "--show-toplevel"]).await;
        let is_root = top.is_ok_and(|out| Path::new(text(&out).trim()) == this.dir);
        if !is_root {
            let branch = format!("--initial-branch={}", this.config.branch);
            let _ = this.git(&["init", "--quiet", &branch]).await?;
        }
        this.exclude_system_dir().await?;
        Ok(this)
    }

    /// The work tree's root directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Commits everything that changed in the work tree as one batch and
    /// returns the commit id, or `None` when nothing changed.
    ///
    /// # Errors
    /// Returns [`StoreError::Backend`] if `git` fails.
    #[tracing::instrument(skip(self), err)]
    pub async fn commit(&self, message: &str) -> StoreResult<Option<String>> {
        let _gate = self.gate.write().await;
        self.commit_all(message).await
    }

    /// Commits the local batch, merges the remote branch through the
    /// three-way merge, and pushes. Conflict copies are named for
    /// `writer` and re-identified with `writer.reidentify`.
    ///
    /// # Errors
    /// Returns [`StoreError::Backend`] if `git` fails (including a fetch or
    /// push the remote refuses for reasons other than a race), or if the
    /// remote keeps moving for five rounds.
    #[tracing::instrument(skip(self, writer), fields(conflicts), err)]
    pub async fn sync(&self, message: &str, writer: &Writer<'_>) -> StoreResult<Synced> {
        let _gate = self.gate.write().await;
        let mut synced = Synced {
            committed: self.commit_all(message).await?,
            ..Synced::default()
        };
        let remote = self.config.remote.as_str();
        if self.git(&["remote", "get-url", remote]).await.is_err() {
            return Ok(synced);
        }
        let branch = format!("refs/heads/{}", self.config.branch);
        for _ in 0..MAX_ROUNDS {
            let fetched = self
                .git(&["fetch", "--quiet", "--no-tags", remote, &branch])
                .await;
            if fetched.is_ok() {
                self.merge_fetched(writer, &mut synced).await?;
            } else if self.remote_has_branch(&branch).await? {
                // The branch exists but could not be fetched.
                let _ = fetched?;
            }
            if self.has_commits().await {
                let refspec = format!("HEAD:{branch}");
                match self
                    .run(&["push", "--quiet", remote, &refspec], None)
                    .await?
                {
                    (true, _, _) => {
                        synced.pushed = true;
                        let _ =
                            tracing::Span::current().record("conflicts", synced.conflicts.len());
                        return Ok(synced);
                    }
                    (false, _, err) if rejected(&err) => {
                        tracing::debug!("remote moved during sync; merging again");
                    }
                    (false, _, err) => return Err(self.failed("push", &err)),
                }
            } else {
                return Ok(synced);
            }
        }
        Err(StoreError::Backend(format!(
            "git: {remote}/{} kept moving; gave up after {MAX_ROUNDS} rounds",
            self.config.branch
        )))
    }

    async fn commit_all(&self, message: &str) -> StoreResult<Option<String>> {
        let _ = self.git(&["add", "--all"]).await?;
        let (clean, _, _) = self.run(&["diff", "--cached", "--quiet"], None).await?;
        if clean && self.has_commits().await {
            return Ok(None);
        }
        let _ = self
            .git(&[
                "commit",
                "--quiet",
                "--no-verify",
                "--allow-empty",
                "-m",
                message,
            ])
            .await?;
        Ok(Some(self.head().await?))
    }

    async fn merge_fetched(&self, writer: &Writer<'_>, synced: &mut Synced) -> StoreResult<()> {
        let before = if self.has_commits().await {
            Some(self.head().await?)
        } else {
            None
        };
        let args = [
            "merge",
            "--quiet",
            "--no-edit",
            "--no-verify",
            "--no-stat",
            "--allow-unrelated-histories",
            "FETCH_HEAD",
        ];
        let (ok, _, err) = self.run(&args, None).await?;
        if !ok {
            let unmerged = self.unmerged().await?;
            if unmerged.is_empty() {
                let _ = self.run(&["merge", "--abort"], None).await;
                return Err(self.failed("merge", &err));
            }
            for entry in unmerged {
                self.resolve(entry, writer, synced).await?;
            }
            let _ = self
                .git(&["commit", "--quiet", "--no-edit", "--no-verify"])
                .await?;
        }
        if let Some(before) = before {
            let out = self
                .git(&["diff", "--name-only", "-z", &before, "HEAD"])
                .await?;
            synced.updated.extend(paths_z(&out));
        } else {
            let out = self.git(&["ls-files", "-z"]).await?;
            synced.updated.extend(paths_z(&out));
        }
        Ok(())
    }

    /// Resolves one conflicted path the way [`crate::save`] does: an edit
    /// beats a delete, clean merges are taken, and a real conflict keeps
    /// the remote version and copies out the local one.
    async fn resolve(
        &self,
        entry: Unmerged,
        writer: &Writer<'_>,
        synced: &mut Synced,
    ) -> StoreResult<()> {
        let path = entry.path.clone();
        let base = match &entry.base {
            Some(oid) => self.blob(oid).await?,
            None => Bytes::new(),
        };
        let ours = match &entry.ours {
            Some(oid) => Some(self.blob(oid).await?),
            None => None,
        };
        let theirs = match &entry.theirs {
            Some(oid) => Some(self.blob(oid).await?),
            None => None,
        };
        match (ours, theirs) {
            (Some(keep), None) | (None, Some(keep)) => {
                let _ = self.local.put(&path, keep, Precondition::None).await?;
            }
            (None, None) => {
                let _ = self.local.delete(&path, Precondition::None).await;
            }
            (Some(ours), Some(theirs)) => {
                let kind = MergeKind::for_path(&path);
                if let Some(merged) = three_way(kind, &base, &ours, &theirs) {
                    let _ = self.local.put(&path, merged, Precondition::None).await?;
                    synced.merged.push(path.clone());
                } else {
                    let _ = self.local.put(&path, theirs, Precondition::None).await?;
                    let copy = self.copy_out(&path, &ours, writer).await?;
                    synced.conflicts.push((path.clone(), copy));
                }
            }
        }
        let _ = self.git(&["add", "--all", "--", &literal(&path)]).await?;
        Ok(())
    }

    async fn copy_out(
        &self,
        path: &BlobPath,
        ours: &[u8],
        writer: &Writer<'_>,
    ) -> StoreResult<BlobPath> {
        let bytes = (writer.reidentify)(ours);
        for n in 1..=MAX_COPIES {
            let copy = conflict_path(path, writer.date, writer.user, n)?;
            match self
                .local
                .put(&copy, bytes.clone(), Precondition::IfAbsent)
                .await
            {
                Ok(_) => {
                    let _ = self.git(&["add", "--", &literal(&copy)]).await?;
                    return Ok(copy);
                }
                Err(StoreError::AlreadyExists(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Err(StoreError::AlreadyExists(path.clone()))
    }

    async fn unmerged(&self) -> StoreResult<Vec<Unmerged>> {
        let out = self.git(&["ls-files", "--unmerged", "-z"]).await?;
        let mut entries: BTreeMap<BlobPath, Unmerged> = BTreeMap::new();
        for record in out.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let line = text(record);
            // "<mode> <oid> <stage>\t<path>"
            let Some((meta, name)) = line.split_once('\t') else {
                continue;
            };
            let mut fields = meta.split(' ');
            let (Some(_mode), Some(oid), Some(stage)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            let Ok(path) = BlobPath::new(name) else {
                continue;
            };
            let entry = entries
                .entry(path.clone())
                .or_insert_with(|| Unmerged::new(path));
            let oid = Some(oid.to_owned());
            match stage {
                "1" => entry.base = oid,
                "2" => entry.ours = oid,
                "3" => entry.theirs = oid,
                _ => {}
            }
        }
        Ok(entries.into_values().collect())
    }

    async fn blob(&self, oid: &str) -> StoreResult<Bytes> {
        Ok(Bytes::from(self.git(&["cat-file", "blob", oid]).await?))
    }

    async fn head(&self) -> StoreResult<String> {
        Ok(text(&self.git(&["rev-parse", "HEAD"]).await?)
            .trim()
            .to_owned())
    }

    async fn has_commits(&self) -> bool {
        self.git(&["rev-parse", "--verify", "--quiet", "HEAD"])
            .await
            .is_ok()
    }

    async fn remote_has_branch(&self, branch: &str) -> StoreResult<bool> {
        let out = self
            .git(&["ls-remote", "--heads", &self.config.remote, branch])
            .await?;
        Ok(!out.is_empty())
    }

    async fn exclude_system_dir(&self) -> StoreResult<()> {
        let path = self
            .git(&["rev-parse", "--git-path", "info/exclude"])
            .await?;
        let file = self.dir.join(text(&path).trim());
        let line = format!("/{SYSTEM_DIR}/");
        let current = tokio::fs::read_to_string(&file).await.unwrap_or_default();
        if current.lines().any(|l| l.trim() == line) {
            return Ok(());
        }
        if let Some(parent) = file.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|source| io(parent, source))?;
        }
        let mut f = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
            .await
            .map_err(|source| io(&file, source))?;
        let entry = format!(
            "{}{line}\n",
            if current.ends_with('\n') || current.is_empty() {
                ""
            } else {
                "\n"
            }
        );
        f.write_all(entry.as_bytes())
            .await
            .map_err(|source| io(&file, source))
    }

    /// Runs `git` and returns stdout, or an error carrying stderr.
    async fn git(&self, args: &[&str]) -> StoreResult<Vec<u8>> {
        match self.run(args, None).await? {
            (true, out, _) => Ok(out),
            (false, _, err) => Err(self.failed(args.first().copied().unwrap_or("git"), &err)),
        }
    }

    /// Runs `git`, returning `(success, stdout, stderr)`.
    async fn run(
        &self,
        args: &[&str],
        stdin: Option<&[u8]>,
    ) -> StoreResult<(bool, Vec<u8>, Vec<u8>)> {
        let mut cmd = Command::new("git");
        let _ = cmd
            .arg("-C")
            .arg(&self.dir)
            .args(["-c", "core.quotepath=off", "-c", "core.hooksPath=/dev/null"])
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", &self.config.author_name)
            .env("GIT_AUTHOR_EMAIL", &self.config.author_email)
            .env("GIT_COMMITTER_NAME", &self.config.author_name)
            .env("GIT_COMMITTER_EMAIL", &self.config.author_email)
            .env("LC_ALL", "C")
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .map_err(|e| StoreError::Backend(format!("git: cannot run git: {e}")))?;
        if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
            pipe.write_all(input)
                .await
                .map_err(|e| StoreError::Backend(format!("git: stdin: {e}")))?;
        }
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| StoreError::Backend(format!("git: {e}")))?;
        Ok((out.status.success(), out.stdout, out.stderr))
    }

    fn failed(&self, what: &str, stderr: &[u8]) -> StoreError {
        let msg = text(stderr);
        let msg = msg.get(..msg.len().min(400)).unwrap_or(&msg).trim();
        StoreError::Backend(format!("git {what} in {}: {msg}", self.dir.display()))
    }

    fn check(path: &BlobPath) -> StoreResult<()> {
        if path.segments().next() == Some(GIT_DIR) {
            return Err(StoreError::InvalidPath {
                path: path.to_string(),
                reason: "is inside the Git directory",
            });
        }
        Ok(())
    }
}

#[derive(Debug)]
struct Unmerged {
    path: BlobPath,
    base: Option<String>,
    ours: Option<String>,
    theirs: Option<String>,
}

impl Unmerged {
    const fn new(path: BlobPath) -> Self {
        Self {
            path,
            base: None,
            ours: None,
            theirs: None,
        }
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn paths_z(out: &[u8]) -> impl Iterator<Item = BlobPath> + '_ {
    out.split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .filter_map(|p| BlobPath::new(text(p)).ok())
}

/// A pathspec that matches `path` exactly, with no glob magic.
fn literal(path: &BlobPath) -> String {
    format!(":(literal){path}")
}

/// True when `git push` failed because the remote moved on.
fn rejected(stderr: &[u8]) -> bool {
    let msg = text(stderr);
    msg.contains("[rejected]") || msg.contains("fetch first") || msg.contains("non-fast-forward")
}

fn io(path: &Path, source: std::io::Error) -> StoreError {
    StoreError::Io {
        path: path.display().to_string(),
        source,
    }
}

#[async_trait]
impl BlobStore for GitStore {
    fn backend(&self) -> &'static str {
        "git"
    }

    async fn get(&self, path: &BlobPath) -> StoreResult<Blob> {
        Self::check(path)?;
        self.local.get(path).await
    }

    async fn head(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        Self::check(path)?;
        self.local.head(path).await
    }

    async fn put(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        condition: Precondition,
    ) -> StoreResult<BlobMeta> {
        Self::check(path)?;
        let _gate = self.gate.read().await;
        self.local.put(path, bytes, condition).await
    }

    async fn delete(&self, path: &BlobPath, condition: Precondition) -> StoreResult<()> {
        Self::check(path)?;
        let _gate = self.gate.read().await;
        self.local.delete(path, condition).await
    }

    async fn rename(&self, from: &BlobPath, to: &BlobPath) -> StoreResult<BlobMeta> {
        Self::check(from)?;
        Self::check(to)?;
        let _gate = self.gate.read().await;
        self.local.rename(from, to).await
    }

    fn list(&self, dir: Option<&BlobPath>) -> BoxStream<'static, StoreResult<BlobEntry>> {
        if let Some(dir) = dir
            && let Err(e) = Self::check(dir)
        {
            return futures::stream::once(async move { Err(e) }).boxed();
        }
        self.local
            .list(dir)
            .filter(|entry| {
                let keep = !matches!(entry, Ok(e) if e.path.segments().next() == Some(GIT_DIR));
                async move { keep }
            })
            .boxed()
    }
}
