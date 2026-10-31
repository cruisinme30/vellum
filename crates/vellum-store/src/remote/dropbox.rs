// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Dropbox adapter (feature `dropbox`), on the Dropbox API v2.
//!
//! Blobs are files beneath a root folder of the user's Dropbox, such as
//! `/Apps/Vellum`. A blob's [`Etag`] is the file's `rev`, and preconditions
//! become Dropbox write modes, which the server checks atomically:
//!
//! | [`Precondition`] | Dropbox                                                   |
//! |------------------|-----------------------------------------------------------|
//! | `None`           | `mode: overwrite`                                         |
//! | `IfAbsent`       | `mode: add`, `strict_conflict: true`                      |
//! | `IfMatch(rev)`   | `mode: {".tag": "update", "update": rev}`, `strict_conflict: true` |
//!
//! `autorename` is always off, so Dropbox never writes a "conflicted copy"
//! behind Vellum's back.
//!
//! # Case-insensitive paths
//!
//! Dropbox paths are case-insensitive: `Notes/a.md` and `notes/A.md` are
//! the same file, and writing one replaces the other. Listings report each
//! file's `path_display`, whose last segment has the casing it was created
//! with; Dropbox does not promise that the casing of parent folders matches
//! the casing other files were written with.
//!
//! # Size limits
//!
//! Blobs up to [`SINGLE_UPLOAD_LIMIT`] bytes go up in one `files/upload`
//! call (Dropbox's documented ceiling for that endpoint is 150 MB); larger
//! blobs use an upload session in chunks of [`UPLOAD_CHUNK`] bytes, which
//! commits atomically under the same write mode.

use std::{
    fmt::Write as _,
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, TryStreamExt, stream::BoxStream};
use reqwest::{RequestBuilder, Response, StatusCode, header::CONTENT_TYPE};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tracing::{Instrument, instrument};

use super::{Http, TokenSource, backend_error};
use crate::{
    error::{StoreError, StoreResult},
    path::{BlobPath, TEMP_PREFIX},
    store::{Blob, BlobEntry, BlobMeta, BlobStore, Etag, Precondition},
};

const BACKEND: &str = "dropbox";
const API_ARG: &str = "Dropbox-API-Arg";
const API_RESULT: &str = "Dropbox-API-Result";

/// Default base URL of the Dropbox RPC endpoints.
pub const DEFAULT_API_URL: &str = "https://api.dropboxapi.com";
/// Default base URL of the Dropbox content (upload and download) endpoints.
pub const DEFAULT_CONTENT_URL: &str = "https://content.dropboxapi.com";
/// Largest blob sent in a single `files/upload` call. Dropbox documents
/// 150 MB as the limit for that endpoint; larger blobs use a session.
pub const SINGLE_UPLOAD_LIMIT: usize = 150 * 1000 * 1000;
/// Chunk size for upload sessions: a multiple of 4,194,304 bytes, as
/// Dropbox asks.
pub const UPLOAD_CHUNK: usize = 64 * 1024 * 1024;

/// Settings for a [`DropboxStore`].
#[derive(Clone, Debug)]
pub struct DropboxConfig {
    /// Base URL of the RPC endpoints, [`DEFAULT_API_URL`] unless testing.
    pub api_url: String,
    /// Base URL of the content endpoints, [`DEFAULT_CONTENT_URL`] unless
    /// testing.
    pub content_url: String,
    /// Folder inside the Dropbox that holds the store, such as
    /// `/Apps/Vellum`, or `""` for the whole Dropbox (or the whole app
    /// folder, for an app-folder token).
    pub root: String,
    /// Where bearer tokens come from.
    pub tokens: Arc<dyn TokenSource>,
    /// Blobs larger than this many bytes use an upload session.
    pub single_upload_limit: usize,
    /// Chunk size for upload sessions.
    pub upload_chunk: usize,
}

impl DropboxConfig {
    /// Settings for the real Dropbox API with the default upload limits.
    #[must_use]
    pub fn new(root: impl Into<String>, tokens: Arc<dyn TokenSource>) -> Self {
        Self {
            api_url: DEFAULT_API_URL.to_owned(),
            content_url: DEFAULT_CONTENT_URL.to_owned(),
            root: root.into(),
            tokens,
            single_upload_limit: SINGLE_UPLOAD_LIMIT,
            upload_chunk: UPLOAD_CHUNK,
        }
    }

    /// Points the store at other base URLs, such as a fake server.
    #[must_use]
    pub fn with_urls(
        mut self,
        api_url: impl Into<String>,
        content_url: impl Into<String>,
    ) -> Self {
        self.api_url = api_url.into();
        self.content_url = content_url.into();
        self
    }
}

/// A [`BlobStore`] backed by a folder in a Dropbox. See the
/// [module docs](self) for how operations map onto the API.
#[derive(Clone, Debug)]
pub struct DropboxStore(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    http: Http,
    api: String,
    content: String,
    root: String,
    root_depth: usize,
    tokens: Arc<dyn TokenSource>,
    single_upload_limit: usize,
    upload_chunk: usize,
}

impl DropboxStore {
    /// Builds a store from `config`.
    ///
    /// # Errors
    /// Returns [`StoreError::InvalidPath`] if `root` is neither empty nor an
    /// absolute Dropbox path, and [`StoreError::Backend`] if the HTTP client
    /// cannot start.
    pub fn new(config: DropboxConfig) -> StoreResult<Self> {
        let root = normalize_root(&config.root)?;
        let root_depth = root.split('/').skip(1).count();
        Ok(Self(Arc::new(Inner {
            http: Http::new(BACKEND)?,
            api: config.api_url.trim_end_matches('/').to_owned(),
            content: config.content_url.trim_end_matches('/').to_owned(),
            root,
            root_depth,
            tokens: config.tokens,
            single_upload_limit: config.single_upload_limit.max(1),
            upload_chunk: config.upload_chunk.max(1),
        })))
    }
}

/// `""` or `/a/b`, with no trailing slash and no empty or relative segments.
fn normalize_root(raw: &str) -> StoreResult<String> {
    let root = raw.trim_end_matches('/');
    let invalid = |reason| {
        Err(StoreError::InvalidPath {
            path: raw.to_owned(),
            reason,
        })
    };
    if root.is_empty() {
        return Ok(String::new());
    }
    if !root.starts_with('/') {
        return invalid("Dropbox root must start with `/`");
    }
    if root
        .split('/')
        .skip(1)
        .any(|s| s.is_empty() || s == "." || s == "..")
    {
        return invalid("Dropbox root has an empty or relative segment");
    }
    Ok(root.to_owned())
}

/// What went wrong with a call: a Dropbox API error (a `409` with an
/// `error_summary`), or anything else, already mapped.
#[derive(Debug)]
enum Fail {
    Api(ApiError),
    Store(StoreError),
}

/// A Dropbox endpoint-specific error, such as `path/not_found/..`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ApiError {
    summary: String,
}

/// The part of an error summary that decides how Vellum reacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    NotFound,
    NotFile,
    NotFolder,
    Conflict,
    InvalidPath,
    Other,
}

/// How a write conflict reads for the operation that hit it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnConflict {
    AlreadyExists,
    PreconditionFailed,
}

impl ApiError {
    /// Parses a `409` body: `{"error_summary": "...", "error": {...}}`.
    fn parse(body: &[u8]) -> Option<Self> {
        let value: Value = serde_json::from_slice(body).ok()?;
        let summary = value.get("error_summary")?.as_str()?;
        Some(Self {
            summary: summary.to_owned(),
        })
    }

    /// Summary segments without Dropbox's trailing `.` padding.
    fn segments(&self) -> impl Iterator<Item = &str> {
        self.summary
            .split('/')
            .map(|s| s.trim_matches('.'))
            .filter(|s| !s.is_empty())
    }

    /// The argument the error is about: `path`, `from_lookup`, `to`, ...
    fn field(&self) -> &str {
        self.segments().next().unwrap_or_default()
    }

    fn reason(&self) -> Reason {
        // Union errors nest one level under the argument (`path/not_found`,
        // `to/conflict/file`); look at every segment after the first.
        self.segments()
            .skip(1)
            .find_map(|s| match s {
                "not_found" => Some(Reason::NotFound),
                "not_file" => Some(Reason::NotFile),
                "not_folder" => Some(Reason::NotFolder),
                "conflict" => Some(Reason::Conflict),
                "malformed_path" | "disallowed_name" => {
                    Some(Reason::InvalidPath)
                }
                _ => None,
            })
            .unwrap_or(Reason::Other)
    }

    /// The [`StoreError`] for this error on `path`.
    fn to_store_error(
        &self,
        path: &BlobPath,
        on_conflict: OnConflict,
    ) -> StoreError {
        match (self.reason(), on_conflict) {
            (Reason::NotFound | Reason::NotFile, _) => {
                StoreError::NotFound(path.clone())
            }
            (Reason::Conflict, OnConflict::AlreadyExists) => {
                StoreError::AlreadyExists(path.clone())
            }
            (Reason::Conflict, OnConflict::PreconditionFailed) => {
                StoreError::PreconditionFailed {
                    path: path.clone(),
                    current: None,
                }
            }
            (Reason::InvalidPath, _) => StoreError::InvalidPath {
                path: path.to_string(),
                reason: "rejected by Dropbox",
            },
            (Reason::NotFolder | Reason::Other, _) => StoreError::Backend(
                format!("{BACKEND}: {} on {path}", self.summary),
            ),
        }
    }
}

impl Fail {
    fn into_store(
        self,
        path: &BlobPath,
        on_conflict: OnConflict,
    ) -> StoreError {
        match self {
            Self::Api(e) => e.to_store_error(path, on_conflict),
            Self::Store(e) => e,
        }
    }
}

/// File metadata as Dropbox returns it. `.tag` is absent where the API
/// returns `FileMetadata` rather than the `Metadata` union.
#[derive(Debug, Deserialize)]
struct Metadata {
    #[serde(rename = ".tag")]
    tag: Option<String>,
    path_display: Option<String>,
    rev: Option<String>,
    size: Option<u64>,
    server_modified: Option<String>,
}

#[derive(Deserialize)]
struct Relocation {
    metadata: Metadata,
}

#[derive(Deserialize)]
struct ListPage {
    entries: Vec<Metadata>,
    cursor: String,
    has_more: bool,
}

#[derive(Deserialize)]
struct SessionStart {
    session_id: String,
}

/// Where a listing has got to.
enum Page {
    First,
    Next(String),
    Done,
}

impl Inner {
    /// The Dropbox path of a blob.
    fn remote(&self, path: &BlobPath) -> String {
        format!("{}/{path}", self.root)
    }

    /// Sends a request with a bearer token. A `401` drops the cached token
    /// and tries once more with a fresh one.
    async fn send(
        &self,
        build: impl Fn(&str) -> RequestBuilder + Send + Sync,
    ) -> StoreResult<Response> {
        for attempt in 1..=2_u32 {
            let token = self.tokens.token().await?;
            let response = self.http.send(|| build(token.expose())).await?;
            if response.status() != StatusCode::UNAUTHORIZED {
                return Ok(response);
            }
            tracing::debug!(
                backend = BACKEND,
                attempt,
                "access token rejected"
            );
            self.tokens.invalidate().await;
        }
        Err(StoreError::Backend(format!(
            "{BACKEND}: access token rejected ({})",
            StatusCode::UNAUTHORIZED
        )))
    }

    /// Passes a success through and turns anything else into a [`Fail`].
    async fn check(
        response: Response,
        path: Option<&BlobPath>,
    ) -> Result<Response, Fail> {
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.bytes().await.unwrap_or_default();
        if status == StatusCode::CONFLICT
            && let Some(e) = ApiError::parse(&body)
        {
            return Err(Fail::Api(e));
        }
        Err(Fail::Store(http_error(path, status, &body)))
    }

    /// Calls an RPC endpoint such as `files/get_metadata`.
    async fn rpc<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        arg: &Value,
        path: Option<&BlobPath>,
    ) -> Result<T, Fail> {
        let url = format!("{}/2/{endpoint}", self.api);
        let response = self
            .send(|token| {
                self.http.client().post(&url).bearer_auth(token).json(arg)
            })
            .await
            .map_err(Fail::Store)?;
        let response = Self::check(response, path).await?;
        response.json().await.map_err(|e| Fail::Store(bad_body(&e)))
    }

    /// Calls a content endpoint such as `files/upload`, with the argument
    /// in the `Dropbox-API-Arg` header.
    async fn content(
        &self,
        endpoint: &str,
        arg: &Value,
        body: Option<Bytes>,
        path: Option<&BlobPath>,
    ) -> Result<Response, Fail> {
        let url = format!("{}/2/{endpoint}", self.content);
        let arg = api_arg(arg);
        let response = self
            .send(|token| {
                let request = self
                    .http
                    .client()
                    .post(&url)
                    .bearer_auth(token)
                    .header(API_ARG, &arg);
                match &body {
                    Some(bytes) => request
                        .header(CONTENT_TYPE, "application/octet-stream")
                        .body(bytes.clone()),
                    None => request,
                }
            })
            .await
            .map_err(Fail::Store)?;
        Self::check(response, path).await
    }

    async fn content_json<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        arg: &Value,
        body: Bytes,
        path: &BlobPath,
    ) -> Result<T, Fail> {
        let response =
            self.content(endpoint, arg, Some(body), Some(path)).await?;
        response.json().await.map_err(|e| Fail::Store(bad_body(&e)))
    }

    async fn head(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        let arg = json!({ "path": self.remote(path) });
        let meta: Metadata = self
            .rpc("files/get_metadata", &arg, Some(path))
            .await
            .map_err(|f| f.into_store(path, OnConflict::AlreadyExists))?;
        file_meta(path, meta)
    }

    /// The error for a failed `IfMatch`: the blob's current version, or
    /// `None` if it is gone.
    async fn precondition_failed(&self, path: &BlobPath) -> StoreError {
        match self.head(path).await {
            Ok(meta) => StoreError::PreconditionFailed {
                path: path.clone(),
                current: Some(meta.etag),
            },
            Err(StoreError::NotFound(_)) => StoreError::PreconditionFailed {
                path: path.clone(),
                current: None,
            },
            Err(e) => e,
        }
    }

    async fn upload(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        commit: &Value,
    ) -> Result<Metadata, Fail> {
        if bytes.len() <= self.single_upload_limit {
            return self
                .content_json("files/upload", commit, bytes, path)
                .await;
        }
        let chunk = self.upload_chunk;
        let first = bytes.slice(..chunk.min(bytes.len()));
        let mut offset = first.len();
        let start: SessionStart = self
            .content_json(
                "files/upload_session/start",
                &json!({ "close": false }),
                first,
                path,
            )
            .await?;
        while bytes.len() - offset > chunk {
            let arg = json!({
                "cursor": { "session_id": start.session_id, "offset": offset },
                "close": false,
            });
            let part = bytes.slice(offset..offset + chunk);
            let _ = self
                .content(
                    "files/upload_session/append_v2",
                    &arg,
                    Some(part),
                    Some(path),
                )
                .await?;
            offset += chunk;
        }
        let arg = json!({
            "cursor": { "session_id": start.session_id, "offset": offset },
            "commit": commit,
        });
        self.content_json(
            "files/upload_session/finish",
            &arg,
            bytes.slice(offset..),
            path,
        )
        .await
    }

    /// One page of a listing, and where the next one starts.
    async fn page(
        &self,
        dir: Option<&BlobPath>,
        page: Page,
    ) -> StoreResult<Option<(Vec<StoreResult<BlobEntry>>, Page)>> {
        let result: Result<ListPage, Fail> = match page {
            Page::Done => return Ok(None),
            Page::First => {
                let path =
                    dir.map_or_else(|| self.root.clone(), |d| self.remote(d));
                let arg = json!({
                    "path": path,
                    "recursive": true,
                    "include_deleted": false,
                    "include_non_downloadable_files": false,
                });
                self.rpc("files/list_folder", &arg, dir).await
            }
            Page::Next(cursor) => {
                let arg = json!({ "cursor": cursor });
                self.rpc("files/list_folder/continue", &arg, dir).await
            }
        };
        match result {
            Ok(list) => {
                let entries = list
                    .entries
                    .into_iter()
                    .filter_map(|m| self.entry(m))
                    .collect();
                let next = if list.has_more {
                    Page::Next(list.cursor)
                } else {
                    Page::Done
                };
                Ok(Some((entries, next)))
            }
            Err(Fail::Api(e)) => match (e.reason(), dir) {
                (Reason::NotFound, _) => Ok(Some((Vec::new(), Page::Done))),
                // `dir` is itself a file: the listing is just that file.
                (Reason::NotFolder, Some(dir)) => {
                    let entry = match self.head(dir).await {
                        Ok(m) => Ok(BlobEntry {
                            path: m.path,
                            size: m.size,
                            modified: m.modified,
                        }),
                        Err(StoreError::NotFound(_)) => {
                            return Ok(Some((Vec::new(), Page::Done)));
                        }
                        Err(e) => Err(e),
                    };
                    Ok(Some((vec![entry], Page::Done)))
                }
                _ => Err(StoreError::Backend(format!(
                    "{BACKEND}: {} while listing",
                    e.summary
                ))),
            },
            Err(Fail::Store(e)) => Err(e),
        }
    }

    /// A listing entry for a file, or `None` for folders, temporary files
    /// and names that are not valid [`BlobPath`]s.
    fn entry(&self, meta: Metadata) -> Option<StoreResult<BlobEntry>> {
        if meta.tag.as_deref() != Some("file") {
            return None;
        }
        let display = meta.path_display.as_deref()?;
        let rel = strip_root(display, self.root_depth)?;
        if rel
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .starts_with(TEMP_PREFIX)
        {
            return None;
        }
        let skip = |e: &StoreError| {
            tracing::warn!(
                backend = BACKEND,
                error = %e,
                "skipping file"
            );
        };
        let path = BlobPath::new(rel).inspect_err(skip).ok()?;
        Some(file_meta(&path, meta).map(|m| BlobEntry {
            path: m.path,
            size: m.size,
            modified: m.modified,
        }))
    }
}

#[async_trait]
impl BlobStore for DropboxStore {
    fn backend(&self) -> &'static str {
        BACKEND
    }

    #[instrument(skip(self), fields(backend = "dropbox", path = %path), err)]
    async fn get(&self, path: &BlobPath) -> StoreResult<Blob> {
        let arg = json!({ "path": self.0.remote(path) });
        let response = self
            .0
            .content("files/download", &arg, None, Some(path))
            .await
            .map_err(|f| f.into_store(path, OnConflict::AlreadyExists))?;
        let header = response
            .headers()
            .get(API_RESULT)
            .map(|v| v.as_bytes().to_vec())
            .ok_or_else(|| {
                StoreError::Backend(format!(
                    "{BACKEND}: no {API_RESULT} header"
                ))
            })?;
        let meta: Metadata = serde_json::from_slice(&header).map_err(|e| {
            StoreError::Backend(format!("{BACKEND}: bad {API_RESULT}: {e}"))
        })?;
        let bytes = response.bytes().await.map_err(|e| bad_body(&e))?;
        let meta = file_meta(path, meta)?;
        Ok(Blob { bytes, meta })
    }

    #[instrument(skip(self), fields(backend = "dropbox", path = %path), err)]
    async fn head(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        self.0.head(path).await
    }

    #[instrument(
        skip(self, bytes),
        fields(backend = "dropbox", path = %path, size = bytes.len()),
        err
    )]
    async fn put(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        condition: Precondition,
    ) -> StoreResult<BlobMeta> {
        let (mode, strict) = match &condition {
            Precondition::None => (json!("overwrite"), false),
            Precondition::IfAbsent => (json!("add"), true),
            Precondition::IfMatch(rev) => {
                (json!({ ".tag": "update", "update": rev.as_str() }), true)
            }
        };
        let commit = json!({
            "path": self.0.remote(path),
            "mode": mode,
            "autorename": false,
            "strict_conflict": strict,
            "mute": true,
        });
        match self.0.upload(path, bytes, &commit).await {
            Ok(meta) => file_meta(path, meta),
            Err(Fail::Api(e))
                if matches!(condition, Precondition::IfMatch(_))
                    && matches!(
                        e.reason(),
                        Reason::Conflict | Reason::NotFound
                    ) =>
            {
                Err(self.0.precondition_failed(path).await)
            }
            Err(f) => Err(f.into_store(path, OnConflict::AlreadyExists)),
        }
    }

    #[instrument(skip(self), fields(backend = "dropbox", path = %path), err)]
    async fn delete(
        &self,
        path: &BlobPath,
        condition: Precondition,
    ) -> StoreResult<()> {
        let remote = self.0.remote(path);
        let arg = match &condition {
            // Fails either way, as the trait documents.
            Precondition::IfAbsent => {
                return match self.0.head(path).await {
                    Ok(_) => Err(StoreError::AlreadyExists(path.clone())),
                    Err(e) => Err(e),
                };
            }
            Precondition::None => {
                // `delete_v2` also removes folders, recursively; make sure
                // the path names a file first.
                let _ = self.0.head(path).await?;
                json!({ "path": remote })
            }
            Precondition::IfMatch(rev) => {
                json!({ "path": remote, "parent_rev": rev.as_str() })
            }
        };
        match self
            .0
            .rpc::<Value>("files/delete_v2", &arg, Some(path))
            .await
        {
            Ok(_) => Ok(()),
            Err(Fail::Api(e))
                if matches!(condition, Precondition::IfMatch(_))
                    && matches!(
                        e.reason(),
                        Reason::Conflict | Reason::NotFound | Reason::NotFile
                    ) =>
            {
                Err(self.0.precondition_failed(path).await)
            }
            Err(f) => Err(f.into_store(path, OnConflict::PreconditionFailed)),
        }
    }

    #[instrument(
        skip(self),
        fields(backend = "dropbox", from = %from, to = %to),
        err
    )]
    async fn rename(
        &self,
        from: &BlobPath,
        to: &BlobPath,
    ) -> StoreResult<BlobMeta> {
        // `move_v2` also moves folders; only blobs (files) may move here.
        let _ = self.0.head(from).await?;
        let arg = json!({
            "from_path": self.0.remote(from),
            "to_path": self.0.remote(to),
            "autorename": false,
            "allow_ownership_transfer": false,
        });
        match self
            .0
            .rpc::<Relocation>("files/move_v2", &arg, Some(from))
            .await
        {
            Ok(moved) => file_meta(to, moved.metadata),
            Err(Fail::Api(e)) => {
                let path = if e.field().starts_with("to") {
                    to
                } else {
                    from
                };
                Err(e.to_store_error(path, OnConflict::AlreadyExists))
            }
            Err(Fail::Store(e)) => Err(e),
        }
    }

    fn list(
        &self,
        dir: Option<&BlobPath>,
    ) -> BoxStream<'static, StoreResult<BlobEntry>> {
        let inner = Arc::clone(&self.0);
        let dir = dir.cloned();
        let span = tracing::info_span!(
            "list",
            backend = BACKEND,
            dir = ?dir.as_ref().map(BlobPath::as_str)
        );
        futures::stream::try_unfold(Page::First, move |page| {
            let (inner, dir, span) =
                (Arc::clone(&inner), dir.clone(), span.clone());
            async move { inner.page(dir.as_ref(), page).instrument(span).await }
        })
        .map_ok(futures::stream::iter)
        .try_flatten()
        .boxed()
    }
}

/// Serializes a `Dropbox-API-Arg` header. HTTP headers must be ASCII, so
/// Dropbox asks for `0x7F` and every non-ASCII character as `\uXXXX`
/// (UTF-16 surrogate pairs beyond the BMP).
fn api_arg(arg: &Value) -> String {
    ascii_json(&arg.to_string())
}

fn ascii_json(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if c.is_ascii() && c != '\u{7f}' {
            out.push(c);
        } else {
            let mut units = [0_u16; 2];
            for unit in c.encode_utf16(&mut units) {
                let _ = write!(out, "\\u{unit:04x}");
            }
        }
    }
    out
}

/// The blob path of a Dropbox `path_display`, dropping the root folder's
/// segments (whatever their casing).
fn strip_root(display: &str, root_depth: usize) -> Option<String> {
    let mut segments = display.strip_prefix('/')?.split('/').skip(root_depth);
    let first = segments.next()?;
    Some(segments.fold(first.to_owned(), |mut rel, s| {
        rel.push('/');
        rel.push_str(s);
        rel
    }))
}

fn file_meta(path: &BlobPath, meta: Metadata) -> StoreResult<BlobMeta> {
    if meta.tag.as_deref().is_some_and(|t| t != "file") {
        return Err(StoreError::NotFound(path.clone()));
    }
    let missing = |field| {
        StoreError::Backend(format!(
            "{BACKEND}: metadata for {path} lacks {field}"
        ))
    };
    let rev = meta.rev.ok_or_else(|| missing("rev"))?;
    let stamp = meta
        .server_modified
        .ok_or_else(|| missing("server_modified"))?;
    let modified = parse_time(&stamp).ok_or_else(|| {
        StoreError::Backend(format!(
            "{BACKEND}: bad server_modified {stamp:?} for {path}"
        ))
    })?;
    Ok(BlobMeta {
        path: path.clone(),
        size: meta.size.ok_or_else(|| missing("size"))?,
        modified,
        etag: Etag::new(rev),
    })
}

/// Parses Dropbox's timestamp format, `%Y-%m-%dT%H:%M:%SZ` (UTC).
fn parse_time(s: &str) -> Option<SystemTime> {
    let num = |from: usize, to: usize| s.get(from..to)?.parse::<u64>().ok();
    let separators = [
        (4, "-"),
        (7, "-"),
        (10, "T"),
        (13, ":"),
        (16, ":"),
        (19, "Z"),
    ];
    if s.len() != 20 || separators.iter().any(|&(i, c)| s.get(i..=i) != Some(c))
    {
        return None;
    }
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
    {
        return None;
    }
    if second > 60 {
        return None;
    }
    let days = u64::try_from(days_from_civil(
        i64::try_from(year).ok()?,
        i64::try_from(month).ok()?,
        i64::try_from(day).ok()?,
    ))
    .ok()?;
    let secs = days * 86_400 + hour * 3_600 + minute * 60 + second;
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(secs))
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_from_march = (month + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day - 1;
    let day_of_era =
        year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// A non-success response that is not a Dropbox API error.
fn http_error(
    path: Option<&BlobPath>,
    status: StatusCode,
    body: &Bytes,
) -> StoreError {
    if let Some(path) = path {
        return backend_error(BACKEND, path, status, body);
    }
    let excerpt = String::from_utf8_lossy(
        body.get(..body.len().min(300)).unwrap_or_default(),
    );
    StoreError::Backend(format!("{BACKEND}: {status}: {}", excerpt.trim()))
}

fn bad_body(e: &reqwest::Error) -> StoreError {
    StoreError::Backend(format!("{BACKEND}: bad response body: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(raw: &str) -> BlobPath {
        BlobPath::new(raw).unwrap_or_else(|e| unreachable!("{e}"))
    }

    fn api(summary: &str) -> ApiError {
        let body =
            json!({ "error_summary": summary, "error": { ".tag": "path" } });
        ApiError::parse(body.to_string().as_bytes())
            .unwrap_or_else(|| unreachable!())
    }

    #[test]
    fn parses_error_summaries() {
        assert_eq!(ApiError::parse(b"not json"), None);
        assert_eq!(ApiError::parse(b"{\"error\": {}}"), None);
        let e = api("path/not_found/...");
        assert_eq!(e.field(), "path");
        assert_eq!(e.reason(), Reason::NotFound);
        assert_eq!(api("path/conflict/file/..").reason(), Reason::Conflict);
        assert_eq!(api("to/conflict/file/.").field(), "to");
        assert_eq!(api("path_lookup/not_found/").reason(), Reason::NotFound);
        assert_eq!(api("path/not_file/..").reason(), Reason::NotFile);
        assert_eq!(api("path/not_folder/.").reason(), Reason::NotFolder);
        assert_eq!(api("path/malformed_path/").reason(), Reason::InvalidPath);
        assert_eq!(api("path/disallowed_name/").reason(), Reason::InvalidPath);
        assert_eq!(api("too_many_write_operations/").reason(), Reason::Other);
        assert_eq!(api("path/insufficient_space/").reason(), Reason::Other);
    }

    #[test]
    fn maps_errors_to_store_errors() {
        let p = path("a/b.md");
        let map = |s: &str, c| api(s).to_store_error(&p, c);
        assert!(matches!(
            map("path/not_found/..", OnConflict::AlreadyExists),
            StoreError::NotFound(q) if q == p
        ));
        assert!(matches!(
            map("path/not_file/..", OnConflict::AlreadyExists),
            StoreError::NotFound(_)
        ));
        assert!(matches!(
            map("path/conflict/file/..", OnConflict::AlreadyExists),
            StoreError::AlreadyExists(q) if q == p
        ));
        assert!(matches!(
            map("path/conflict/file/..", OnConflict::PreconditionFailed),
            StoreError::PreconditionFailed { current: None, .. }
        ));
        assert!(matches!(
            map("path/malformed_path/", OnConflict::AlreadyExists),
            StoreError::InvalidPath { .. }
        ));
        let other =
            map("path/insufficient_space/..", OnConflict::AlreadyExists);
        assert!(
            matches!(&other, StoreError::Backend(m) if m.contains("insufficient\
                _space")),
            "{other:?}"
        );
    }

    #[test]
    fn http_errors_never_echo_headers() {
        let body = Bytes::from_static(b"Error in call to API function");
        let e = http_error(None, StatusCode::BAD_REQUEST, &body);
        assert!(
            matches!(&e, StoreError::Backend(m) if m.contains("400")),
            "{e:?}"
        );
        let e = http_error(Some(&path("x")), StatusCode::BAD_REQUEST, &body);
        assert!(
            matches!(&e, StoreError::Backend(m) if m.contains("on x")),
            "{e:?}"
        );
    }

    #[test]
    fn api_args_are_ascii() {
        let arg = api_arg(&json!({ "path": "/Apps/Vellum/Notes – ü.md" }));
        assert_eq!(arg, r#"{"path":"/Apps/Vellum/Notes \u2013 \u00fc.md"}"#);
        assert_eq!(ascii_json("😀\u{7f}"), r"\ud83d\ude00\u007f");
        let back: Value =
            serde_json::from_str(&ascii_json("\"😀 ü\"")).unwrap_or_default();
        assert_eq!(back, json!("😀 ü"));
    }

    #[test]
    fn parses_dropbox_timestamps() {
        let t = parse_time("2015-05-12T15:50:38Z");
        let want = SystemTime::UNIX_EPOCH + Duration::from_secs(1_431_445_838);
        assert_eq!(t, Some(want));
        assert_eq!(
            parse_time("1970-01-01T00:00:00Z"),
            Some(SystemTime::UNIX_EPOCH)
        );
        let leap = SystemTime::UNIX_EPOCH + Duration::from_hours(11_016 * 24);
        assert_eq!(parse_time("2000-02-29T00:00:00Z"), Some(leap));
        for bad in [
            "",
            "2015-05-12 15:50:38Z",
            "2015-05-12T15:50:38",
            "2015-13-12T15:50:38Z",
            "1969-12-31T23:59:59Z",
            "2015-05-12T15:50:38.123Z",
        ] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn roots_are_normalized() {
        assert_eq!(normalize_root("").ok(), Some(String::new()));
        assert_eq!(normalize_root("/").ok(), Some(String::new()));
        assert_eq!(
            normalize_root("/Apps/Vellum/").ok(),
            Some("/Apps/Vellum".to_owned())
        );
        for bad in ["Apps", "/a//b", "/a/../b", "/a/./b"] {
            assert!(normalize_root(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn strips_the_root_whatever_its_case() {
        assert_eq!(
            strip_root("/apps/VELLUM/Notes/a.md", 2).as_deref(),
            Some("Notes/a.md")
        );
        assert_eq!(strip_root("/a.md", 0).as_deref(), Some("a.md"));
        assert_eq!(strip_root("/Apps/Vellum", 2), None);
        assert_eq!(strip_root("relative", 0), None);
    }

    #[test]
    fn only_files_become_blobs() {
        let p = path("a.md");
        let folder = Metadata {
            tag: Some("folder".to_owned()),
            path_display: Some("/a.md".to_owned()),
            rev: None,
            size: None,
            server_modified: None,
        };
        assert!(matches!(
            file_meta(&p, folder),
            Err(StoreError::NotFound(_))
        ));
        let file = Metadata {
            tag: None,
            path_display: None,
            rev: Some("015f1c2b3d4e5f6a7".to_owned()),
            size: Some(3),
            server_modified: Some("2026-01-02T03:04:05Z".to_owned()),
        };
        let meta = file_meta(&p, file);
        assert!(
            matches!(
                &meta,
                Ok(m) if m.etag.as_str() == "015f1c2b3d4e5f6a7" && m.size == 3
            ),
            "{meta:?}"
        );
    }
}
