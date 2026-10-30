// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `OneDrive` and `SharePoint` document libraries over Microsoft Graph.
//!
//! Blobs are driveItems addressed by path beneath a root folder of the
//! drive (`{drive}/root:/{root}/{path}:`, each segment percent-encoded).
//! Graph checks `If-Match` and `@microsoft.graph.conflictBehavior=fail`
//! server side, so preconditions hold against every other writer, not just
//! this process.
//!
//! # Versions
//!
//! The [`Etag`] is the driveItem's `eTag`, not its `cTag`. Graph documents
//! `If-Match` against the `eTag` for uploads, upload sessions and deletes,
//! and every upload response carries it, so the version we hand out is
//! exactly the one Graph checks. The `cTag` tracks content only, so it would
//! survive a rename, but it is not returned for every item and Graph does
//! not promise to compare it in `If-Match` everywhere. The cost of `eTag`
//! is that a rename or other metadata change also yields a new version: a
//! writer holding the old etag gets `PreconditionFailed` and merges, which
//! is safe.
//!
//! # Downloads
//!
//! Content is fetched from the item's pre-authenticated
//! `@microsoft.graph.downloadUrl` rather than by following the `302` from
//! `/content`: reqwest keeps the `Authorization` header on a redirect to the
//! same host, so following it could hand the bearer token to a download
//! host. The download request is built without credentials, and the URL
//! itself is never logged or put in an error.

use std::{
    collections::VecDeque,
    fmt::Write as _,
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, stream::BoxStream};
use reqwest::{
    RequestBuilder, Response, StatusCode,
    header::{CONTENT_RANGE, CONTENT_TYPE, IF_MATCH},
};
use serde::{Deserialize, de::IgnoredAny};
use serde_json::json;
use tracing::{Instrument, instrument};
use vellum_kernel::Timestamp;

use super::{Http, TokenSource, backend_error};
use crate::{
    error::{StoreError, StoreResult},
    path::{BlobPath, TEMP_PREFIX},
    store::{Blob, BlobEntry, BlobMeta, BlobStore, Etag, Precondition},
};

const BACKEND: &str = "onedrive";

/// The Microsoft Graph v1.0 endpoint.
pub const DEFAULT_BASE_URL: &str = "https://graph.microsoft.com/v1.0";

/// Largest blob sent as one simple upload; larger ones use an upload
/// session.
pub const SIMPLE_UPLOAD_MAX: usize = 4 * 1024 * 1024;

/// Default upload-session chunk: ten mebibytes, a multiple of the
/// 320-kibibyte unit Graph requires for every chunk but the last.
pub const UPLOAD_CHUNK: usize = 32 * 320 * 1024;

/// Fields a listing needs; the rest of a driveItem is left out.
const LIST_SELECT: &str =
    "$select=id,name,size,lastModifiedDateTime,file,folder";

/// Which drive holds the blobs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drive {
    /// The signed-in user's `OneDrive` (`/me/drive`).
    Me,
    /// A drive by id (`/drives/{id}`), such as a `SharePoint` document
    /// library.
    Id(String),
}

/// Settings for a [`OneDriveStore`].
#[derive(Clone, Debug)]
pub struct OneDriveConfig {
    /// Graph base URL, [`DEFAULT_BASE_URL`] unless testing.
    pub base_url: String,
    /// The drive to use.
    pub drive: Drive,
    /// `/`-separated folder inside the drive that holds the store; empty
    /// for the drive root.
    pub root: String,
    /// Bearer tokens for Graph.
    pub tokens: Arc<dyn TokenSource>,
    /// Largest blob sent as a simple upload ([`SIMPLE_UPLOAD_MAX`]).
    pub simple_upload_max: usize,
    /// Upload-session chunk size ([`UPLOAD_CHUNK`]). Graph needs a
    /// multiple of 320 kibibytes.
    pub chunk_size: usize,
}

impl OneDriveConfig {
    /// Settings for `root` in `drive` against the public Graph endpoint.
    #[must_use]
    pub fn new(
        drive: Drive,
        root: impl Into<String>,
        tokens: Arc<dyn TokenSource>,
    ) -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            drive,
            root: root.into(),
            tokens,
            simple_upload_max: SIMPLE_UPLOAD_MAX,
            chunk_size: UPLOAD_CHUNK,
        }
    }
}

/// A [`BlobStore`] backed by a folder in `OneDrive` or a `SharePoint`
/// document library, through Microsoft Graph.
#[derive(Clone, Debug)]
pub struct OneDriveStore(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    http: Http,
    /// The configured base URL, without a trailing slash.
    base: String,
    /// `{base}/me/drive` or `{base}/drives/{id}`.
    drive: String,
    /// Root folder segments, unencoded.
    root: Vec<String>,
    tokens: Arc<dyn TokenSource>,
    simple_max: usize,
    chunk: usize,
}

impl OneDriveStore {
    /// Builds a store from `config`. Nothing is sent until the first call.
    ///
    /// # Errors
    /// Returns [`StoreError::InvalidPath`] if `config.root` has an empty,
    /// `.` or `..` segment or a control character, and
    /// [`StoreError::Backend`] if the HTTP client cannot start.
    pub fn new(config: OneDriveConfig) -> StoreResult<Self> {
        let root = root_segments(&config.root)?;
        let base = config.base_url.trim_end_matches('/').to_owned();
        let drive = match &config.drive {
            Drive::Me => format!("{base}/me/drive"),
            Drive::Id(id) => format!("{base}/drives/{}", encode(id)),
        };
        Ok(Self(Arc::new(Inner {
            http: Http::new(BACKEND)?,
            base,
            drive,
            root,
            tokens: config.tokens,
            simple_max: config.simple_upload_max,
            chunk: config.chunk_size.max(1),
        })))
    }
}

fn root_segments(root: &str) -> StoreResult<Vec<String>> {
    let trimmed = root.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let invalid = |reason| StoreError::InvalidPath {
        path: root.to_owned(),
        reason,
    };
    if trimmed.chars().any(|c| c == '\\' || c.is_control()) {
        return Err(invalid("backslash or control character"));
    }
    trimmed
        .split('/')
        .map(|s| match s {
            "" => Err(invalid("empty segment")),
            "." | ".." => Err(invalid("relative segment")),
            s => Ok(s.to_owned()),
        })
        .collect()
}

#[async_trait]
impl BlobStore for OneDriveStore {
    fn backend(&self) -> &'static str {
        BACKEND
    }

    #[instrument(skip(self), fields(backend = "onedrive", path = %path), err)]
    async fn get(&self, path: &BlobPath) -> StoreResult<Blob> {
        self.0.get(path).await
    }

    #[instrument(skip(self), fields(backend = "onedrive", path = %path), err)]
    async fn head(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        self.0.meta(path).await
    }

    #[instrument(
        skip(self, bytes),
        fields(backend = "onedrive", path = %path, size = bytes.len()),
        err
    )]
    async fn put(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        cond: Precondition,
    ) -> StoreResult<BlobMeta> {
        let uploaded = if bytes.len() <= self.0.simple_max {
            self.0.simple_upload(path, bytes, &cond).await
        } else {
            self.0.session_upload(path, bytes, &cond).await
        };
        match uploaded {
            Ok(item) => file_meta(path, &item),
            Err(e) => Err(self.0.refine(path, &cond, e).await),
        }
    }

    #[instrument(skip(self), fields(backend = "onedrive", path = %path), err)]
    async fn delete(
        &self,
        path: &BlobPath,
        cond: Precondition,
    ) -> StoreResult<()> {
        match &cond {
            Precondition::IfAbsent => match self.0.meta(path).await {
                Ok(_) => Err(StoreError::AlreadyExists(path.clone())),
                Err(e) => Err(e),
            },
            // Deleting a folder would remove everything in it, so make sure
            // the path is a file first. With `IfMatch` a folder never
            // matches a file's eTag, so Graph refuses on its own.
            Precondition::None => {
                let _ = self.0.meta(path).await?;
                self.0.remove(path, &cond).await
            }
            Precondition::IfMatch(_) => {
                match self.0.remove(path, &cond).await {
                    Ok(()) => Ok(()),
                    Err(e) => Err(self.0.refine(path, &cond, e).await),
                }
            }
        }
    }

    #[instrument(
        skip(self),
        fields(backend = "onedrive", from = %from, to = %to),
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
            backend = BACKEND,
            dir = ?dir.map(BlobPath::as_str)
        );
        let walk = Walk {
            inner: Arc::clone(&self.0),
            dir: dir.cloned(),
            begun: false,
            pages: Vec::new(),
            ready: VecDeque::new(),
        };
        futures::stream::try_unfold(walk, move |mut walk| {
            async move {
                let next = walk.next().await?;
                Ok(next.map(|entry| (entry, walk)))
            }
            .instrument(span.clone())
        })
        .boxed()
    }
}

impl Inner {
    /// The driveItem URL for `segments` beneath the drive root.
    fn url_for(&self, segments: &[&str]) -> String {
        if segments.is_empty() {
            return format!("{}/root", self.drive);
        }
        let mut url = format!("{}/root:", self.drive);
        for segment in segments {
            url.push('/');
            url.push_str(&encode(segment));
        }
        url.push(':');
        url
    }

    /// The driveItem URL for a blob, or for the store root with `None`.
    fn item_url(&self, path: Option<&BlobPath>) -> String {
        let segments: Vec<&str> = self
            .root
            .iter()
            .map(String::as_str)
            .chain(path.into_iter().flat_map(BlobPath::segments))
            .collect();
        self.url_for(&segments)
    }

    fn children_url(&self, id: &str) -> String {
        format!("{}/items/{}/children?{LIST_SELECT}", self.drive, encode(id))
    }

    /// Sends an authenticated Graph request. A `401` drops the cached token
    /// and retries once with a fresh one; a second `401` is returned to the
    /// caller, which reports it as [`StoreError::Backend`].
    async fn send(
        &self,
        build: impl Fn() -> RequestBuilder + Send + Sync,
    ) -> StoreResult<Response> {
        let token = self.tokens.token().await?;
        let response = self
            .http
            .send(|| build().bearer_auth(token.expose()))
            .await?;
        if response.status() != StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        tracing::debug!(backend = BACKEND, "access token rejected, refreshing");
        self.tokens.invalidate().await;
        let token = self.tokens.token().await?;
        self.http.send(|| build().bearer_auth(token.expose())).await
    }

    async fn item(&self, path: &BlobPath) -> StoreResult<Item> {
        let url = self.item_url(Some(path));
        let response = self.send(|| self.http.client().get(&url)).await?;
        item_response(path, response).await
    }

    async fn meta(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        file_meta(path, &self.item(path).await?)
    }

    async fn get(&self, path: &BlobPath) -> StoreResult<Blob> {
        // Metadata first, content second: if the blob changes in between we
        // return newer bytes under the older etag, so the next conditional
        // write fails and merges instead of losing an update.
        let item = self.item(path).await?;
        let mut meta = file_meta(path, &item)?;
        let url = item.download_url.ok_or_else(|| {
            StoreError::Backend(format!(
                "{BACKEND}: {path} has no download URL"
            ))
        })?;
        // No bearer token: the URL is pre-authenticated.
        let response = self.http.send(|| self.http.client().get(&url)).await?;
        if !response.status().is_success() {
            return Err(graph_error(path, response).await);
        }
        let bytes = response.bytes().await.map_err(|e| {
            StoreError::Backend(format!(
                "{BACKEND}: download of {path}: {}",
                e.without_url()
            ))
        })?;
        meta.size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        Ok(Blob { bytes, meta })
    }

    async fn simple_upload(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        cond: &Precondition,
    ) -> StoreResult<Item> {
        let url = format!(
            "{}/content?@microsoft.graph.conflictBehavior={}",
            self.item_url(Some(path)),
            conflict_behavior(cond)
        );
        let response = self
            .send(|| {
                let request = self
                    .http
                    .client()
                    .put(&url)
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .body(bytes.clone());
                if_match(request, cond)
            })
            .await?;
        item_response(path, response).await
    }

    /// Uploads through `createUploadSession` in `chunk`-sized ranges. Graph
    /// checks `If-Match` and the conflict behaviour when the session is
    /// created and again when the last range commits the file.
    async fn session_upload(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        cond: &Precondition,
    ) -> StoreResult<Item> {
        let url = format!("{}/createUploadSession", self.item_url(Some(path)));
        let body = json!({
            "item": {
                "@microsoft.graph.conflictBehavior": conflict_behavior(cond)
            }
        });
        let response = self
            .send(|| if_match(self.http.client().post(&url).json(&body), cond))
            .await?;
        if !response.status().is_success() {
            return Err(graph_error(path, response).await);
        }
        let session: UploadSession =
            response.json().await.map_err(|e| decode_error(path, e))?;
        let outcome =
            self.upload_chunks(path, &session.upload_url, &bytes).await;
        if outcome.is_err() {
            // Best effort: an abandoned session expires on its own anyway.
            let _ = self.http.client().delete(&session.upload_url).send().await;
        }
        outcome
    }

    async fn upload_chunks(
        &self,
        path: &BlobPath,
        upload: &str,
        bytes: &Bytes,
    ) -> StoreResult<Item> {
        let total = bytes.len();
        let mut start = 0;
        while start < total {
            let end = start.saturating_add(self.chunk).min(total);
            let part = bytes.slice(start..end);
            let range = format!("bytes {start}-{}/{total}", end - 1);
            // No bearer token: the upload URL is pre-authenticated, and
            // Graph rejects requests to it that carry one.
            let response = self
                .http
                .send(|| {
                    self.http
                        .client()
                        .put(upload)
                        .header(CONTENT_RANGE, &range)
                        .body(part.clone())
                })
                .await?;
            let status = response.status();
            if end < total && status == StatusCode::ACCEPTED {
                start = end;
            } else if end == total
                && status.is_success()
                && status != StatusCode::ACCEPTED
            {
                return item_response(path, response).await;
            } else {
                return Err(graph_error(path, response).await);
            }
        }
        Err(StoreError::Backend(format!(
            "{BACKEND}: upload session for {path} ended without a file"
        )))
    }

    async fn remove(
        &self,
        path: &BlobPath,
        cond: &Precondition,
    ) -> StoreResult<()> {
        let url = self.item_url(Some(path));
        let response = self
            .send(|| if_match(self.http.client().delete(&url), cond))
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(graph_error(path, response).await)
        }
    }

    async fn rename(
        &self,
        from: &BlobPath,
        to: &BlobPath,
    ) -> StoreResult<BlobMeta> {
        // Only files move: patching a folder path would move a whole tree.
        let _ = self.meta(from).await?;
        let to_parent = to.parent();
        let parent: Vec<&str> = self
            .root
            .iter()
            .map(String::as_str)
            .chain(to_parent.iter().flat_map(BlobPath::segments))
            .collect();
        let parent_id = self.ensure_folder(to, &parent).await?;
        let url = format!(
            "{}?@microsoft.graph.conflictBehavior=fail",
            self.item_url(Some(from))
        );
        // `parentReference.path` is read-only in Graph; moves go by id.
        let body = json!({
            "parentReference": { "id": parent_id },
            "name": to.file_name()
        });
        let response = self
            .send(|| self.http.client().patch(&url).json(&body))
            .await?;
        if !response.status().is_success() {
            return Err(match graph_error(from, response).await {
                StoreError::AlreadyExists(_) => {
                    StoreError::AlreadyExists(to.clone())
                }
                e => e,
            });
        }
        let item: Item =
            response.json().await.map_err(|e| decode_error(to, e))?;
        file_meta(to, &item)
    }

    /// The id of the folder at `segments`, creating it and any missing
    /// ancestors. `ctx` names the blob the folder is for, in errors.
    async fn ensure_folder(
        &self,
        ctx: &BlobPath,
        segments: &[&str],
    ) -> StoreResult<String> {
        let mut existing = segments.len();
        let mut id = loop {
            match self
                .folder_id(ctx, segments.get(..existing).unwrap_or_default())
                .await
            {
                Ok(id) => break id,
                Err(StoreError::NotFound(_)) if existing > 0 => existing -= 1,
                Err(e) => return Err(e),
            }
        };
        for (i, name) in segments.iter().enumerate().skip(existing) {
            id = match self.create_folder(ctx, &id, name).await {
                Ok(id) => id,
                // Another writer created it first.
                Err(StoreError::AlreadyExists(_)) => {
                    self.folder_id(ctx, segments.get(..=i).unwrap_or_default())
                        .await?
                }
                Err(e) => return Err(e),
            };
        }
        Ok(id)
    }

    async fn folder_id(
        &self,
        ctx: &BlobPath,
        segments: &[&str],
    ) -> StoreResult<String> {
        let url = self.url_for(segments);
        let response = self.send(|| self.http.client().get(&url)).await?;
        let item = item_response(ctx, response).await?;
        if item.folder.is_some() {
            Ok(item.id)
        } else {
            Err(StoreError::Backend(format!(
                "{BACKEND}: a file is in the way of {ctx}"
            )))
        }
    }

    async fn create_folder(
        &self,
        ctx: &BlobPath,
        parent: &str,
        name: &str,
    ) -> StoreResult<String> {
        let url = format!("{}/items/{}/children", self.drive, encode(parent));
        let body = json!({
            "name": name,
            "folder": {},
            "@microsoft.graph.conflictBehavior": "fail",
        });
        let response = self
            .send(|| self.http.client().post(&url).json(&body))
            .await?;
        Ok(item_response(ctx, response).await?.id)
    }

    /// Turns a failed conditional write or delete into the error the
    /// [`BlobStore`] contract promises, fetching the current etag for
    /// [`StoreError::PreconditionFailed`].
    async fn refine(
        &self,
        path: &BlobPath,
        cond: &Precondition,
        err: StoreError,
    ) -> StoreError {
        match (cond, err) {
            (Precondition::IfAbsent, StoreError::PreconditionFailed { .. }) => {
                StoreError::AlreadyExists(path.clone())
            }
            (
                Precondition::IfMatch(_),
                StoreError::PreconditionFailed { .. } | StoreError::NotFound(_),
            ) => match self.meta(path).await {
                Ok(m) => StoreError::PreconditionFailed {
                    path: path.clone(),
                    current: Some(m.etag),
                },
                Err(StoreError::NotFound(_)) => {
                    StoreError::PreconditionFailed {
                        path: path.clone(),
                        current: None,
                    }
                }
                Err(e) => e,
            },
            (_, e) => e,
        }
    }
}

/// A recursive listing in progress: a stack of children pages still to
/// fetch and the entries of the last page not yet yielded.
struct Walk {
    inner: Arc<Inner>,
    /// The directory to list, `None` for the whole store.
    dir: Option<BlobPath>,
    /// Whether the directory itself has been fetched yet.
    begun: bool,
    /// Page URLs still to fetch, each with the blob path of its folder.
    pages: Vec<(String, String)>,
    ready: VecDeque<BlobEntry>,
}

impl Walk {
    async fn next(&mut self) -> StoreResult<Option<BlobEntry>> {
        loop {
            if let Some(entry) = self.ready.pop_front() {
                return Ok(Some(entry));
            }
            if !self.begun {
                self.begun = true;
                self.begin().await?;
                continue;
            }
            let Some((url, prefix)) = self.pages.pop() else {
                return Ok(None);
            };
            self.page(&url, &prefix).await?;
        }
    }

    async fn begin(&mut self) -> StoreResult<()> {
        let dir = self.dir.take();
        let url = self.inner.item_url(dir.as_ref());
        let response = self
            .inner
            .send(|| self.inner.http.client().get(&url))
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        let item: Item = listing_json(response).await?;
        if item.folder.is_some() {
            let prefix = dir.map(|d| d.to_string()).unwrap_or_default();
            self.pages.push((self.inner.children_url(&item.id), prefix));
        } else if let Some(dir) = dir
            && item.file.is_some()
        {
            self.ready.push_back(entry(dir, &item)?);
        }
        Ok(())
    }

    async fn page(&mut self, url: &str, prefix: &str) -> StoreResult<()> {
        let response = self
            .inner
            .send(|| self.inner.http.client().get(url))
            .await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(()); // The folder went away mid-walk.
        }
        let page: Page = listing_json(response).await?;
        for child in page.value {
            if child.name.starts_with(TEMP_PREFIX) {
                continue;
            }
            let rel = if prefix.is_empty() {
                child.name.clone()
            } else {
                format!("{prefix}/{}", child.name)
            };
            if child.folder.is_some() {
                self.pages.push((self.inner.children_url(&child.id), rel));
            } else if child.file.is_some() {
                match BlobPath::new(rel) {
                    Ok(path) => self.ready.push_back(entry(path, &child)?),
                    Err(e) => {
                        tracing::warn!(
                            backend = BACKEND,
                            error = %e,
                            "skipping item"
                        )
                    }
                }
            }
        }
        if let Some(next) = page.next_link {
            // Only ever send the bearer token back to Graph itself.
            if !next.starts_with(&self.inner.base) {
                return Err(StoreError::Backend(format!(
                    "{BACKEND}: listing page link points outside Graph"
                )));
            }
            self.pages.push((next, prefix.to_owned()));
        }
        Ok(())
    }
}

/// The parts of a driveItem the adapter reads. No `Debug`: the download
/// URL is a credential.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(rename = "eTag")]
    e_tag: Option<String>,
    size: Option<u64>,
    last_modified_date_time: Option<String>,
    file: Option<IgnoredAny>,
    folder: Option<IgnoredAny>,
    #[serde(rename = "@microsoft.graph.downloadUrl")]
    download_url: Option<String>,
}

#[derive(Deserialize)]
struct Page {
    value: Vec<Item>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadSession {
    upload_url: String,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Deserialize)]
struct ErrorDetail {
    code: String,
}

const fn conflict_behavior(cond: &Precondition) -> &'static str {
    match cond {
        Precondition::IfAbsent => "fail",
        Precondition::None | Precondition::IfMatch(_) => "replace",
    }
}

fn if_match(request: RequestBuilder, cond: &Precondition) -> RequestBuilder {
    match cond {
        Precondition::IfMatch(etag) => request.header(IF_MATCH, etag.as_str()),
        Precondition::None | Precondition::IfAbsent => request,
    }
}

async fn item_response(
    path: &BlobPath,
    response: Response,
) -> StoreResult<Item> {
    if response.status().is_success() {
        response.json().await.map_err(|e| decode_error(path, e))
    } else {
        Err(graph_error(path, response).await)
    }
}

/// Maps a Graph error (`{"error": {"code": ...}}`) onto [`StoreError`].
async fn graph_error(path: &BlobPath, response: Response) -> StoreError {
    let status = response.status();
    let body = response.bytes().await.unwrap_or_default();
    let code = serde_json::from_slice::<ErrorBody>(&body)
        .ok()
        .map(|b| b.error.code);
    match (status, code.as_deref()) {
        (StatusCode::NOT_FOUND, _) | (_, Some("itemNotFound")) => {
            StoreError::NotFound(path.clone())
        }
        (_, Some("nameAlreadyExists")) => {
            StoreError::AlreadyExists(path.clone())
        }
        (StatusCode::PRECONDITION_FAILED, _) => {
            StoreError::PreconditionFailed {
                path: path.clone(),
                current: None,
            }
        }
        _ => backend_error(BACKEND, path, status, &body),
    }
}

async fn listing_json<T: for<'de> Deserialize<'de>>(
    response: Response,
) -> StoreResult<T> {
    let status = response.status();
    if status.is_success() {
        return response.json().await.map_err(|e| {
            StoreError::Backend(format!(
                "{BACKEND}: bad listing: {}",
                e.without_url()
            ))
        });
    }
    let body = response.bytes().await.unwrap_or_default();
    let excerpt = String::from_utf8_lossy(
        body.get(..body.len().min(300)).unwrap_or_default(),
    );
    Err(StoreError::Backend(format!(
        "{BACKEND}: {status} while listing: {}",
        excerpt.trim()
    )))
}

fn decode_error(path: &BlobPath, e: reqwest::Error) -> StoreError {
    StoreError::Backend(format!(
        "{BACKEND}: bad response for {path}: {}",
        e.without_url()
    ))
}

fn file_meta(path: &BlobPath, item: &Item) -> StoreResult<BlobMeta> {
    if item.file.is_none() {
        return Err(StoreError::NotFound(path.clone()));
    }
    let etag = item.e_tag.clone().ok_or_else(|| {
        StoreError::Backend(format!("{BACKEND}: {path} has no eTag"))
    })?;
    Ok(BlobMeta {
        path: path.clone(),
        size: item.size.unwrap_or_default(),
        modified: modified(&path.to_string(), item)?,
        etag: Etag::new(etag),
    })
}

fn entry(path: BlobPath, item: &Item) -> StoreResult<BlobEntry> {
    let modified = modified(path.as_str(), item)?;
    Ok(BlobEntry {
        path,
        size: item.size.unwrap_or_default(),
        modified,
    })
}

/// Parses `lastModifiedDateTime` (RFC 3339) to millisecond precision.
fn modified(path: &str, item: &Item) -> StoreResult<SystemTime> {
    let bad = || {
        StoreError::Backend(format!(
            "{BACKEND}: bad lastModifiedDateTime on {path}"
        ))
    };
    let stamp: Timestamp = item
        .last_modified_date_time
        .as_deref()
        .ok_or_else(bad)?
        .parse()
        .map_err(|_| bad())?;
    let ms = stamp.unix_ms();
    let offset = Duration::from_millis(ms.unsigned_abs());
    let time = if ms >= 0 {
        SystemTime::UNIX_EPOCH.checked_add(offset)
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(offset)
    };
    time.ok_or_else(bad)
}

/// Percent-encodes one path segment, keeping only RFC 3986 unreserved
/// characters.
fn encode(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::StaticToken;

    fn store(drive: Drive, root: &str) -> StoreResult<OneDriveStore> {
        let mut config =
            OneDriveConfig::new(drive, root, Arc::new(StaticToken::new("t")));
        config.base_url = "https://graph.example/v1.0/".to_owned();
        OneDriveStore::new(config)
    }

    #[test]
    fn encodes_segments() {
        assert_eq!(encode("Notes – ü.md"), "Notes%20%E2%80%93%20%C3%BC.md");
        assert_eq!(encode("a:b#c?d%"), "a%3Ab%23c%3Fd%25");
    }

    #[test]
    fn addresses_items_by_path() -> StoreResult<()> {
        let s = store(Drive::Me, "/Apps/Vellum/")?;
        let p = BlobPath::new("a b/c.md")?;
        assert_eq!(
            s.0.item_url(Some(&p)),
            "https://graph.example/v1.0/me/drive/root:/Apps/Vellum/a%20b/c.md:"
        );
        let s = store(Drive::Id("b!x/y".to_owned()), "")?;
        assert_eq!(
            s.0.item_url(None),
            "https://graph.example/v1.0/drives/b%21x%2Fy/root"
        );
        Ok(())
    }

    #[test]
    fn rejects_bad_roots() {
        for bad in ["a//b", "a/../b", "./a", "a\\b"] {
            assert!(store(Drive::Me, bad).is_err(), "{bad:?}");
        }
    }
}
