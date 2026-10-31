// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Drive API v3 calls: bearer authentication with one retry after a
//! `401`, bounded retries on Drive's `403` throttling, and the file
//! endpoints the adapter uses (metadata, download, list, create, update,
//! delete, multipart and resumable uploads).

use std::{
    fmt::Write as _,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bytes::{Bytes, BytesMut};
use reqwest::{
    RequestBuilder, Response, StatusCode,
    header::{CONTENT_RANGE, CONTENT_TYPE, LOCATION, RANGE},
};
use serde::Deserialize;
use serde_json::json;

use super::time::parse_rfc3339;
use crate::{
    error::{StoreError, StoreResult},
    path::BlobPath,
    remote::{Http, TokenSource, backend_error, status_error},
    store::{BlobMeta, Etag},
};

/// Backend name for spans, metrics and error messages.
pub(super) const BACKEND: &str = "gdrive";
/// The `mimeType` Drive gives folders.
pub(super) const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
/// Prefix of Google-native types (Docs, Sheets, shortcuts, ...), which have
/// no downloadable bytes.
const NATIVE_PREFIX: &str = "application/vnd.google-apps.";
/// The file fields every call asks for.
const FIELDS: &str = "id,name,mimeType,parents,size,modifiedTime,createdTime,ve\
                      rsion,md5Checksum,headRevisionId,trashed";
/// Attempts when Drive answers `403 rateLimitExceeded`, including the first.
const RATE_LIMIT_ATTEMPTS: u32 = 5;
/// First delay after a `403` rate limit; doubles on each retry.
const RATE_LIMIT_BACKOFF: Duration = Duration::from_millis(250);
/// Resumable chunks must be multiples of this (Drive's rule).
pub(super) const CHUNK_GRANULE: usize = 256 * 1024;
/// Resumable rounds that make no progress before giving up.
const MAX_STALLS: u32 = 3;

/// A Drive file resource, limited to [`FIELDS`]. Drive encodes 64-bit
/// numbers (`size`, `version`) as JSON strings.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct DriveFile {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) mime_type: String,
    pub(super) parents: Vec<String>,
    pub(super) size: Option<String>,
    pub(super) modified_time: Option<String>,
    pub(super) created_time: Option<String>,
    pub(super) version: Option<String>,
    pub(super) md5_checksum: Option<String>,
    pub(super) head_revision_id: Option<String>,
    pub(super) trashed: bool,
}

impl DriveFile {
    pub(super) fn is_folder(&self) -> bool {
        self.mime_type == FOLDER_MIME
    }

    /// Google Docs, Sheets, Slides, shortcuts and the like: no bytes.
    pub(super) fn is_native(&self) -> bool {
        self.mime_type.starts_with(NATIVE_PREFIX)
    }

    pub(super) fn size(&self) -> u64 {
        self.size
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or_default()
    }

    pub(super) fn modified(&self) -> SystemTime {
        self.modified_time
            .as_deref()
            .and_then(parse_rfc3339)
            .unwrap_or(UNIX_EPOCH)
    }

    /// Sort key for picking among same-named siblings: oldest first, then
    /// by id, so every process picks the same one.
    pub(super) fn age_key(&self) -> (SystemTime, &str) {
        let created = self.created_time.as_deref().and_then(parse_rfc3339);
        (created.unwrap_or(UNIX_EPOCH), self.id.as_str())
    }

    /// The content version.
    ///
    /// `headRevisionId` names the revision holding the current bytes: Drive
    /// creates a revision on every content upload and on nothing else, so
    /// it changes exactly when the content does and survives renames,
    /// moves, sharing and other metadata edits. `version` is the fallback
    /// for files Drive reports without a head revision: it increases on
    /// every change, content or metadata, so it never misses a content
    /// change but may move without one, which costs at most a spurious
    /// precondition failure and a merge, never a lost update.
    pub(super) fn etag(&self, path: &BlobPath) -> StoreResult<Etag> {
        match (&self.head_revision_id, &self.version) {
            (Some(rev), _) => Ok(Etag::new(format!("rev:{rev}"))),
            (None, Some(version)) => Ok(Etag::new(format!("ver:{version}"))),
            (None, None) => Err(StoreError::Backend(format!(
                "{BACKEND}: {path} has neither headRevisionId nor version"
            ))),
        }
    }

    pub(super) fn meta(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        Ok(BlobMeta {
            path: path.clone(),
            size: self.size(),
            modified: self.modified(),
            etag: self.etag(path)?,
        })
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct FileList {
    pub(super) files: Vec<DriveFile>,
    pub(super) next_page_token: Option<String>,
    pub(super) incomplete_search: bool,
}

/// How a `files.list` query should match `mimeType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Folder,
    NotFolder,
    Any,
}

/// The Drive API endpoints, authenticated and throttle-aware.
#[derive(Debug)]
pub(super) struct Client {
    http: Http,
    api: String,
    upload: String,
    tokens: Arc<dyn TokenSource>,
    drive_id: Option<String>,
    page_size: u32,
    resumable_threshold: usize,
    chunk: usize,
}

/// Escapes a string literal for a Drive `q` query.
pub(super) fn escape_query(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Drive ids are URL-safe; refuse anything else before putting one in a
/// URL path.
fn checked_id(id: &str) -> StoreResult<&str> {
    let ok = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(id)
    } else {
        Err(StoreError::Backend(format!(
            "{BACKEND}: unexpected file id {id:?}"
        )))
    }
}

/// The content type stored with an upload. Drive converts only when asked,
/// so this only affects how Drive's own UI previews the file.
fn mime_for(name: &str) -> &'static str {
    let markdown = std::path::Path::new(name).extension().is_some_and(|e| {
        e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown")
    });
    if markdown {
        "text/markdown"
    } else {
        "application/octet-stream"
    }
}

/// Whether a `403` body is Drive's rate limit rather than a permission error.
fn is_rate_limit(body: &[u8]) -> bool {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Envelope {
        error: Inner,
    }
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Inner {
        errors: Vec<Reason>,
        details: Vec<Reason>,
    }
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Reason {
        reason: String,
    }
    let Ok(envelope) = serde_json::from_slice::<Envelope>(body) else {
        return false;
    };
    let inner = envelope.error;
    inner.errors.iter().chain(&inner.details).any(|r| {
        matches!(
            r.reason.as_str(),
            "rateLimitExceeded"
                | "userRateLimitExceeded"
                | "RATE_LIMIT_EXCEEDED"
        )
    })
}

/// A [`StoreError::Backend`] for a failed call, naming the path when
/// there is one.
fn body_error(
    path: Option<&BlobPath>,
    what: &str,
    status: StatusCode,
    body: &Bytes,
) -> StoreError {
    path.map_or_else(
        || {
            let excerpt = String::from_utf8_lossy(
                body.get(..body.len().min(300)).unwrap_or_default(),
            );
            StoreError::Backend(format!(
                "{BACKEND}: {status} on {what}: {}",
                excerpt.trim()
            ))
        },
        |path| backend_error(BACKEND, path, status, body),
    )
}

/// Turns a non-success response into an error: `404` is `NotFound` when
/// there is a path to name, everything else is `Backend`.
async fn failure(
    response: Response,
    path: Option<&BlobPath>,
    what: &str,
) -> StoreError {
    if let Some(path) = path {
        return status_error(BACKEND, path, response).await;
    }
    let status = response.status();
    let body = response.bytes().await.unwrap_or_default();
    body_error(None, what, status, &body)
}

async fn success(
    response: Response,
    path: Option<&BlobPath>,
    what: &str,
) -> StoreResult<Response> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(failure(response, path, what).await)
    }
}

async fn json<T: for<'de> Deserialize<'de>>(
    response: Response,
    what: &str,
) -> StoreResult<T> {
    response.json().await.map_err(|e| {
        StoreError::Backend(format!(
            "{BACKEND}: bad response for {what}: {}",
            e.without_url()
        ))
    })
}

/// A `multipart/related` body: JSON metadata, then the media.
fn multipart(
    metadata: &serde_json::Value,
    media: &[u8],
    mime: &str,
) -> (String, Bytes) {
    let mut boundary = String::from("vellum-gdrive-boundary");
    let mut n = 0_u32;
    // The boundary must not occur inside the media.
    while media
        .windows(boundary.len())
        .any(|w| w == boundary.as_bytes())
    {
        n += 1;
        boundary.truncate("vellum-gdrive-boundary".len());
        let _ = write!(boundary, "-{n}");
    }
    let mut body = BytesMut::with_capacity(media.len() + 256);
    let head = format!(
        "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n\
         {metadata}\r\n--{boundary}\r\nContent-Type: {mime}\r\n\r\n"
    );
    body.extend_from_slice(head.as_bytes());
    body.extend_from_slice(media);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary, body.freeze())
}

/// The next byte offset Drive expects, from a `308`'s `Range: bytes=0-N`.
fn received(response: &Response) -> usize {
    response
        .headers()
        .get(RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().strip_prefix("bytes=0-"))
        .and_then(|end| end.parse::<usize>().ok())
        .map_or(0, |end| end + 1)
}

impl Client {
    pub(super) fn new(
        api: &str,
        upload: &str,
        tokens: Arc<dyn TokenSource>,
        drive_id: Option<String>,
        page_size: u32,
        resumable_threshold: usize,
        chunk: usize,
    ) -> StoreResult<Self> {
        // Round the chunk down to Drive's granule, keeping at least one.
        let chunk = (chunk / CHUNK_GRANULE).max(1) * CHUNK_GRANULE;
        Ok(Self {
            http: Http::new(BACKEND)?,
            api: api.trim_end_matches('/').to_owned(),
            upload: upload.trim_end_matches('/').to_owned(),
            tokens,
            drive_id,
            page_size: page_size.clamp(1, 1000),
            resumable_threshold,
            chunk,
        })
    }

    /// Sends an authenticated request. A `401` drops the cached token and
    /// retries once; a `403` rate limit backs off and retries a few times
    /// ([`Http::send`] already retries `429` and `5xx`). Any other `403` is
    /// returned as [`StoreError::Backend`]; other statuses are returned.
    async fn call(
        &self,
        path: Option<&BlobPath>,
        what: &str,
        build: impl Fn(&reqwest::Client) -> RequestBuilder + Send + Sync,
    ) -> StoreResult<Response> {
        let mut reauthed = false;
        let mut throttled = 0;
        let mut delay = RATE_LIMIT_BACKOFF;
        loop {
            let token = self.tokens.token().await?;
            let response = self
                .http
                .send(|| build(self.http.client()).bearer_auth(token.expose()))
                .await?;
            match response.status() {
                StatusCode::UNAUTHORIZED if !reauthed => {
                    tracing::debug!(
                        backend = BACKEND,
                        "access token rejected; refreshing"
                    );
                    self.tokens.invalidate().await;
                    reauthed = true;
                }
                StatusCode::UNAUTHORIZED => {
                    return Err(StoreError::Backend(format!(
                        "{BACKEND}: 401 Unauthorized on {what}: credentials \
                            rejected twice"
                    )));
                }
                StatusCode::FORBIDDEN => {
                    let body = response.bytes().await.unwrap_or_default();
                    throttled += 1;
                    if !is_rate_limit(&body) || throttled >= RATE_LIMIT_ATTEMPTS
                    {
                        return Err(body_error(
                            path,
                            what,
                            StatusCode::FORBIDDEN,
                            &body,
                        ));
                    }
                    tracing::debug!(
                        backend = BACKEND,
                        attempt = throttled,
                        ?delay,
                        "rate limited"
                    );
                    tokio::time::sleep(delay).await;
                    delay = delay.saturating_mul(2);
                }
                _ => return Ok(response),
            }
        }
    }

    fn file_url(&self, id: &str) -> StoreResult<String> {
        Ok(format!("{}/files/{}", self.api, checked_id(id)?))
    }

    /// `files.get` metadata.
    pub(super) async fn get_meta(
        &self,
        id: &str,
        path: Option<&BlobPath>,
    ) -> StoreResult<DriveFile> {
        let url = self.file_url(id)?;
        let response = self
            .call(path, "files.get", |c| {
                c.get(&url)
                    .query(&[("fields", FIELDS), ("supportsAllDrives", "true")])
            })
            .await?;
        json(success(response, path, "files.get").await?, "files.get").await
    }

    /// `files.get?alt=media`: the file's bytes.
    pub(super) async fn download(
        &self,
        id: &str,
        path: &BlobPath,
    ) -> StoreResult<Bytes> {
        let url = self.file_url(id)?;
        let response = self
            .call(Some(path), "files.get", |c| {
                c.get(&url)
                    .query(&[("alt", "media"), ("supportsAllDrives", "true")])
            })
            .await?;
        success(response, Some(path), "files.get")
            .await?
            .bytes()
            .await
            .map_err(|e| {
                StoreError::Backend(format!(
                    "{BACKEND}: reading {path}: {}",
                    e.without_url()
                ))
            })
    }

    /// One page of the non-trashed children of `parent`, optionally only
    /// those named `name` and of `kind`.
    pub(super) async fn list_page(
        &self,
        parent: &str,
        name: Option<&str>,
        kind: Kind,
        page_token: Option<&str>,
    ) -> StoreResult<FileList> {
        let mut q = String::new();
        if let Some(name) = name {
            let _ = write!(q, "name = '{}' and ", escape_query(name));
        }
        let _ = write!(
            q,
            "'{}' in parents and trashed = false",
            escape_query(parent)
        );
        match kind {
            Kind::Folder => {
                let _ = write!(q, " and mimeType = '{FOLDER_MIME}'");
            }
            Kind::NotFolder => {
                let _ = write!(q, " and mimeType != '{FOLDER_MIME}'");
            }
            Kind::Any => {}
        }
        let fields = format!("nextPageToken,incompleteSearch,files({FIELDS})");
        let page_size = self.page_size.to_string();
        let url = format!("{}/files", self.api);
        let response = self
            .call(None, "files.list", |c| {
                let mut req = c.get(&url).query(&[
                    ("q", q.as_str()),
                    ("fields", fields.as_str()),
                    ("pageSize", page_size.as_str()),
                    ("supportsAllDrives", "true"),
                    ("includeItemsFromAllDrives", "true"),
                ]);
                req = match &self.drive_id {
                    Some(drive) => req.query(&[
                        ("corpora", "drive"),
                        ("driveId", drive.as_str()),
                    ]),
                    None => req.query(&[("corpora", "user")]),
                };
                if let Some(token) = page_token {
                    req = req.query(&[("pageToken", token)]);
                }
                req
            })
            .await?;
        let page: FileList =
            json(success(response, None, "files.list").await?, "files.list")
                .await?;
        if page.incomplete_search {
            tracing::warn!(
                backend = BACKEND,
                "files.list reported an incomplete search"
            );
        }
        Ok(page)
    }

    /// Every non-trashed child of `parent` named `name` and of `kind`,
    /// across all pages.
    pub(super) async fn find(
        &self,
        parent: &str,
        name: &str,
        kind: Kind,
    ) -> StoreResult<Vec<DriveFile>> {
        let mut found = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self
                .list_page(parent, Some(name), kind, token.as_deref())
                .await?;
            // `name =` matches exactly, but be strict in case of folding.
            found.extend(
                page.files
                    .into_iter()
                    .filter(|f| f.name == name && !f.trashed),
            );
            match page.next_page_token {
                Some(next) if !next.is_empty() => token = Some(next),
                _ => return Ok(found),
            }
        }
    }

    /// `files.create` for a folder.
    pub(super) async fn create_folder(
        &self,
        parent: &str,
        name: &str,
    ) -> StoreResult<DriveFile> {
        let url = format!("{}/files", self.api);
        let body = json!({
            "name": name,
            "mimeType": FOLDER_MIME,
            "parents": [parent]
        });
        let response = self
            .call(None, "files.create", |c| {
                c.post(&url)
                    .query(&[("fields", FIELDS), ("supportsAllDrives", "true")])
                    .json(&body)
            })
            .await?;
        json(
            success(response, None, "files.create").await?,
            "files.create",
        )
        .await
    }

    /// Creates a file in `parent`: a multipart upload, or a resumable one
    /// above the threshold.
    pub(super) async fn create_file(
        &self,
        parent: &str,
        path: &BlobPath,
        bytes: Bytes,
    ) -> StoreResult<DriveFile> {
        let name = path.file_name();
        let mime = mime_for(name);
        let metadata = json!({ "name": name, "parents": [parent] });
        let url = format!("{}/files", self.upload);
        if self.resumable(&bytes) {
            return self
                .resumable_upload(path, mime, bytes, |c| {
                    c.post(&url)
                        .query(&[
                            ("uploadType", "resumable"),
                            ("fields", FIELDS),
                            ("supportsAllDrives", "true"),
                        ])
                        .json(&metadata)
                })
                .await;
        }
        let (boundary, body) = multipart(&metadata, &bytes, mime);
        let content_type = format!("multipart/related; boundary={boundary}");
        let response = self
            .call(Some(path), "files.create", |c| {
                c.post(&url)
                    .query(&[
                        ("uploadType", "multipart"),
                        ("fields", FIELDS),
                        ("supportsAllDrives", "true"),
                    ])
                    .header(CONTENT_TYPE, &content_type)
                    .body(body.clone())
            })
            .await?;
        json(
            success(response, Some(path), "files.create").await?,
            "files.create",
        )
        .await
    }

    /// Replaces a file's content: a media upload, or a resumable one above
    /// the threshold.
    pub(super) async fn update_content(
        &self,
        id: &str,
        path: &BlobPath,
        bytes: Bytes,
    ) -> StoreResult<DriveFile> {
        let mime = mime_for(path.file_name());
        let url = format!("{}/files/{}", self.upload, checked_id(id)?);
        if self.resumable(&bytes) {
            let empty = json!({});
            return self
                .resumable_upload(path, mime, bytes, |c| {
                    c.patch(&url)
                        .query(&[
                            ("uploadType", "resumable"),
                            ("fields", FIELDS),
                            ("supportsAllDrives", "true"),
                        ])
                        .json(&empty)
                })
                .await;
        }
        let response = self
            .call(Some(path), "files.update", |c| {
                c.patch(&url)
                    .query(&[
                        ("uploadType", "media"),
                        ("fields", FIELDS),
                        ("supportsAllDrives", "true"),
                    ])
                    .header(CONTENT_TYPE, mime)
                    .body(bytes.clone())
            })
            .await?;
        json(
            success(response, Some(path), "files.update").await?,
            "files.update",
        )
        .await
    }

    const fn resumable(&self, bytes: &Bytes) -> bool {
        !bytes.is_empty() && bytes.len() > self.resumable_threshold
    }

    /// Starts a resumable session with `start`, then sends the content in
    /// chunks, resuming from wherever Drive says it got to.
    async fn resumable_upload(
        &self,
        path: &BlobPath,
        mime: &str,
        bytes: Bytes,
        start: impl Fn(&reqwest::Client) -> RequestBuilder + Send + Sync,
    ) -> StoreResult<DriveFile> {
        let total = bytes.len();
        let length = total.to_string();
        let response = self
            .call(Some(path), "resumable upload", |c| {
                start(c)
                    .header("X-Upload-Content-Type", mime)
                    .header("X-Upload-Content-Length", &length)
            })
            .await?;
        let response =
            success(response, Some(path), "resumable upload").await?;
        // The session URI authorises uploads by itself: never log it.
        let session = response
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .ok_or_else(|| {
                StoreError::Backend(format!(
                    "{BACKEND}: resumable upload of {path} has no session"
                ))
            })?;
        let mut offset: usize = 0;
        let mut stalls = 0;
        loop {
            let end = offset.saturating_add(self.chunk).min(total);
            let part = bytes.slice(offset..end);
            // With everything sent, ask Drive to finish the upload.
            let range = if offset == total {
                format!("bytes */{total}")
            } else {
                format!("bytes {offset}-{}/{total}", end.saturating_sub(1))
            };
            let response = self
                .call(Some(path), "resumable upload", |c| {
                    c.put(&session)
                        .header(CONTENT_RANGE, &range)
                        .body(part.clone())
                })
                .await?;
            match response.status() {
                StatusCode::OK | StatusCode::CREATED => {
                    return json(response, "resumable upload").await;
                }
                StatusCode::PERMANENT_REDIRECT => {
                    let next = received(&response).min(total);
                    if next <= offset {
                        stalls += 1;
                        if stalls >= MAX_STALLS {
                            return Err(StoreError::Backend(format!(
                                "{BACKEND}: resumable upload of {path} stalled \
                                    at byte {offset}"
                            )));
                        }
                    } else {
                        stalls = 0;
                    }
                    tracing::debug!(
                        backend = BACKEND,
                        received = next,
                        total,
                        "upload chunk sent"
                    );
                    offset = next;
                }
                _ => {
                    return Err(failure(
                        response,
                        Some(path),
                        "resumable upload",
                    )
                    .await);
                }
            }
        }
    }

    /// `files.update` of metadata: a new name, and a move between parents.
    pub(super) async fn move_file(
        &self,
        id: &str,
        path: &BlobPath,
        name: &str,
        add_parent: Option<&str>,
        remove_parent: Option<&str>,
    ) -> StoreResult<DriveFile> {
        let url = self.file_url(id)?;
        let body = json!({ "name": name });
        let response = self
            .call(Some(path), "files.update", |c| {
                let mut req = c
                    .patch(&url)
                    .query(&[("fields", FIELDS), ("supportsAllDrives", "true")])
                    .json(&body);
                if let Some(add) = add_parent {
                    req = req.query(&[("addParents", add)]);
                }
                if let Some(remove) = remove_parent {
                    req = req.query(&[("removeParents", remove)]);
                }
                req
            })
            .await?;
        json(
            success(response, Some(path), "files.update").await?,
            "files.update",
        )
        .await
    }

    /// `files.delete`, or moving the file to Drive's trash.
    pub(super) async fn remove(
        &self,
        id: &str,
        path: Option<&BlobPath>,
        to_trash: bool,
    ) -> StoreResult<()> {
        let url = self.file_url(id)?;
        let response = if to_trash {
            let body = json!({ "trashed": true });
            self.call(path, "files.update", |c| {
                c.patch(&url)
                    .query(&[("supportsAllDrives", "true")])
                    .json(&body)
            })
            .await?
        } else {
            self.call(path, "files.delete", |c| {
                c.delete(&url).query(&[("supportsAllDrives", "true")])
            })
            .await?
        };
        let _ = success(response, path, "files.delete").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_query_literals() {
        assert_eq!(escape_query(r"it's a\b"), r"it\'s a\\b");
    }

    #[test]
    fn recognises_rate_limits() {
        let limited = br#"{"error":{"code":403,"errors":[{"reason":"userRateLimitExceeded"}]}}"#;
        let denied =
            br#"{"error":{"code":403,"errors":[{"reason":"insufficientFilePermissions"}]}}"#;
        assert!(is_rate_limit(limited));
        assert!(!is_rate_limit(denied));
        assert!(!is_rate_limit(b"not json"));
    }

    #[test]
    fn multipart_boundary_avoids_the_media() {
        let media = b"--vellum-gdrive-boundary";
        let (boundary, body) = multipart(&json!({}), media, "text/plain");
        assert_eq!(boundary, "vellum-gdrive-boundary-1");
        assert!(body.ends_with(b"\r\n--vellum-gdrive-boundary-1--\r\n"));
    }

    #[test]
    fn rejects_odd_ids() {
        assert!(checked_id("1AbC-_9").is_ok());
        assert!(checked_id("../x").is_err());
        assert!(checked_id("").is_err());
    }
}
