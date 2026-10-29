// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Amazon S3 and S3-compatible object stores (`MinIO`, R2, `LocalStack`).
//!
//! Every [`BlobPath`] maps to the object key `<prefix>/<path>` in one
//! bucket. Requests are signed with `SigV4` (see `sigv4`), sent through
//! [`Http::send`] for retries, and preconditions map onto S3's conditional
//! headers, so the bucket itself arbitrates racing writers:
//!
//! | Precondition | `PutObject`         | Refused with             |
//! |--------------|---------------------|--------------------------|
//! | `IfAbsent`   | `If-None-Match: *`  | `412`/`409` → `AlreadyExists` |
//! | `IfMatch(e)` | `If-Match: e`       | `412`/`409` → `PreconditionFailed`, `404` → `PreconditionFailed { current: None }` |
//!
//! Etags are the S3 `ETag` header exactly as sent, quotes included.
//!
//! A conditional request that succeeded but whose response was lost is
//! retried by [`Http::send`] and then refused (the object now matches the
//! new version, not the old one), so a caller can see `AlreadyExists` or
//! `PreconditionFailed` for its own write. The merge path re-reads and
//! finds its own content, so nothing is lost.

mod credentials;
mod list;
mod sigv4;

pub use credentials::{Container, CredentialSource, InstanceProfile, WebIdentity};

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, stream::BoxStream};
use reqwest::{
    Method, Response, StatusCode, Url,
    header::{AUTHORIZATION, CONTENT_LENGTH, DATE, ETAG, HeaderMap, LAST_MODIFIED},
};
use tracing::{Instrument, instrument};

use super::{Http, Secret, status_error};
use crate::{
    error::{StoreError, StoreResult},
    path::BlobPath,
    store::{Blob, BlobEntry, BlobMeta, BlobStore, Etag, Precondition},
};

const BACKEND: &str = "s3";

/// How the bucket appears in request URLs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Addressing {
    /// `https://<bucket>.<endpoint host>/<key>`: what AWS recommends.
    #[default]
    VirtualHosted,
    /// `https://<endpoint host>/<bucket>/<key>`: for `MinIO`, `LocalStack`
    /// and endpoints without wildcard DNS.
    Path,
}

/// Static AWS credentials. The secret parts never appear in `Debug` output.
#[derive(Clone, Debug)]
pub struct S3Credentials {
    /// The access key id, such as `AKIAIOSFODNN7EXAMPLE`.
    pub access_key_id: String,
    /// The secret access key.
    pub secret_access_key: Secret,
    /// The session token for temporary (STS) credentials.
    pub session_token: Option<Secret>,
}

/// Where an [`S3Store`] keeps its blobs and how it signs in.
#[derive(Clone, Debug)]
pub struct S3Config {
    /// The service endpoint without the bucket, such as
    /// `https://s3.eu-west-1.amazonaws.com` or `http://127.0.0.1:9000`.
    pub endpoint: String,
    /// The signing region, such as `eu-west-1`.
    pub region: String,
    /// The bucket name.
    pub bucket: String,
    /// Optional key prefix: blob `a/b.md` is stored at `<prefix>/a/b.md`.
    /// It must itself be a valid [`BlobPath`].
    pub prefix: Option<String>,
    /// Virtual-hosted or path-style URLs.
    pub addressing: Addressing,
    /// Where the signing keys come from: static keys, or the pod's or
    /// instance's own identity.
    pub credentials: CredentialSource,
}

/// Region used when `VELLUM_S3_REGION` is unset.
pub const DEFAULT_REGION: &str = "us-east-1";

impl S3Config {
    /// Reads the configuration from the process environment; see
    /// [`S3Config::from_vars`] for the variables.
    ///
    /// # Errors
    /// As [`S3Config::from_vars`].
    pub fn from_env() -> StoreResult<Self> {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    /// Builds the configuration from the deploy contract's variables,
    /// looked up through `var` (empty values count as unset):
    ///
    /// - `VELLUM_BLOB_URL` (required): `s3://bucket` or
    ///   `s3://bucket/optional/prefix`.
    /// - `VELLUM_S3_ENDPOINT`: a custom endpoint such as `MinIO` or
    ///   `LocalStack`, addressed path-style. Unset means AWS's regional
    ///   endpoint `https://s3.<region>.amazonaws.com`, addressed
    ///   virtual-hosted style.
    /// - `VELLUM_S3_REGION`: defaults to [`DEFAULT_REGION`].
    /// - The credentials, as [`CredentialSource::from_vars`] reads them:
    ///   `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`, or workload
    ///   identity (IRSA, EKS Pod Identity, ECS, an EC2 instance profile).
    ///
    /// # Errors
    /// Returns [`StoreError::Backend`] naming a missing variable, or saying
    /// why `VELLUM_BLOB_URL` is malformed. Secret values are never quoted.
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> StoreResult<Self> {
        let get = |name: &str| var(name).filter(|v| !v.is_empty());
        let require = |name: &str| {
            get(name).ok_or_else(|| {
                StoreError::Backend(format!("s3: environment variable {name} is not set"))
            })
        };
        let url = require("VELLUM_BLOB_URL")?;
        let bad_url = |why: &str| StoreError::Backend(format!("s3: VELLUM_BLOB_URL {why}"));
        let rest = url
            .strip_prefix("s3://")
            .ok_or_else(|| bad_url("must start with s3://"))?;
        let (bucket, prefix) = rest.split_once('/').unwrap_or((rest, ""));
        if bucket.is_empty() {
            return Err(bad_url("names no bucket"));
        }
        let prefix = prefix.trim_end_matches('/');
        let region = get("VELLUM_S3_REGION").unwrap_or_else(|| DEFAULT_REGION.to_owned());
        let (endpoint, addressing) = get("VELLUM_S3_ENDPOINT").map_or_else(
            || {
                (
                    format!("https://s3.{region}.amazonaws.com"),
                    Addressing::VirtualHosted,
                )
            },
            |endpoint| (endpoint.trim_end_matches('/').to_owned(), Addressing::Path),
        );
        let on_aws = addressing == Addressing::VirtualHosted;
        let credentials = CredentialSource::from_vars(&var, &region, on_aws)?;
        Ok(Self {
            endpoint,
            region,
            bucket: bucket.to_owned(),
            prefix: (!prefix.is_empty()).then(|| prefix.to_owned()),
            addressing,
            credentials,
        })
    }
}

/// A [`BlobStore`] backed by one S3 bucket, optionally under a key prefix.
///
/// Cheap to clone; clones share the HTTP client.
#[derive(Clone, Debug)]
pub struct S3Store(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    http: Http,
    /// `scheme://host[:port]`, bucket included for virtual-hosted URLs.
    origin: String,
    /// The `Host` header that reqwest will send for `origin`.
    host: String,
    /// The encoded path of the bucket root: `/<bucket>` or empty.
    bucket_path: String,
    prefix: Option<BlobPath>,
    region: String,
    credentials: credentials::Provider,
    /// Set once the bucket answered `501` to `If-Match` on `DeleteObject`.
    no_conditional_delete: AtomicBool,
}

/// One request before signing.
struct Call<'a> {
    method: Method,
    /// The object key, or `None` for the bucket itself.
    key: Option<&'a str>,
    query: &'a [(&'a str, &'a str)],
    headers: Vec<(&'static str, String)>,
    body: Bytes,
}

impl<'a> Call<'a> {
    const fn object(method: Method, key: &'a str) -> Self {
        Self {
            method,
            key: Some(key),
            query: &[],
            headers: Vec::new(),
            body: Bytes::new(),
        }
    }

    fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }
}

impl S3Store {
    /// Builds a store for `config`. Makes no request: a wrong bucket or bad
    /// credentials surface on the first operation.
    ///
    /// # Errors
    /// Returns [`StoreError::Backend`] when the endpoint is not an `http`
    /// or `https` URL without a path, query or credentials, or the bucket
    /// name cannot appear in a host name, and
    /// [`StoreError::InvalidPath`] when the prefix is not a valid
    /// [`BlobPath`].
    pub fn new(config: S3Config) -> StoreResult<Self> {
        let invalid = |why: &str| StoreError::Backend(format!("s3: invalid endpoint: {why}"));
        let url = Url::parse(&config.endpoint).map_err(|e| invalid(&e.to_string()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(invalid("scheme must be http or https"));
        }
        if url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
            return Err(invalid("must not have a path, query or fragment"));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(invalid("must not embed credentials"));
        }
        let host = url.host_str().ok_or_else(|| invalid("no host"))?;
        let bucket = &config.bucket;
        let bucket_ok = !bucket.is_empty()
            && bucket
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.'));
        if !bucket_ok {
            return Err(StoreError::Backend(format!(
                "s3: invalid bucket name {bucket:?}"
            )));
        }
        let host = match config.addressing {
            Addressing::VirtualHosted => format!("{bucket}.{host}"),
            Addressing::Path => host.to_owned(),
        };
        let host = url
            .port()
            .map_or_else(|| host.clone(), |p| format!("{host}:{p}"));
        let origin = format!("{}://{host}", url.scheme());
        let bucket_path = match config.addressing {
            Addressing::VirtualHosted => String::new(),
            Addressing::Path => format!("/{}", sigv4::uri_encode(bucket, false)),
        };
        let prefix = config.prefix.map(BlobPath::new).transpose()?;
        Ok(Self(Arc::new(Inner {
            http: Http::new(BACKEND)?,
            origin,
            host,
            bucket_path,
            prefix,
            region: config.region,
            credentials: credentials::Provider::new(config.credentials)?,
            no_conditional_delete: AtomicBool::new(false),
        })))
    }

    /// The object key for `path`, prefix included.
    fn key(&self, path: &BlobPath) -> String {
        self.0.key(path)
    }

    /// The current etag at `path`, or `None` when nothing is stored there.
    async fn current(&self, path: &BlobPath) -> StoreResult<Option<Etag>> {
        match self.head(path).await {
            Ok(meta) => Ok(Some(meta.etag)),
            Err(StoreError::NotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The non-atomic `IfMatch` check for buckets without conditional
    /// deletes: fails unless the current etag is `want`.
    async fn check_then(&self, path: &BlobPath, want: &Etag) -> StoreResult<()> {
        let current = self.current(path).await?;
        if current.as_ref() == Some(want) {
            Ok(())
        } else {
            Err(StoreError::PreconditionFailed {
                path: path.clone(),
                current,
            })
        }
    }

    /// Maps a `DeleteObject` response.
    async fn deleted(&self, path: &BlobPath, response: Response) -> StoreResult<()> {
        match response.status() {
            s if s.is_success() => Ok(()),
            StatusCode::NOT_FOUND => Err(StoreError::PreconditionFailed {
                path: path.clone(),
                current: None,
            }),
            StatusCode::PRECONDITION_FAILED | StatusCode::CONFLICT => Err(self.changed(path).await),
            _ => Err(error(path, response).await),
        }
    }

    /// A [`StoreError::PreconditionFailed`] carrying the current etag.
    async fn changed(&self, path: &BlobPath) -> StoreError {
        match self.current(path).await {
            Ok(current) => StoreError::PreconditionFailed {
                path: path.clone(),
                current,
            },
            Err(e) => e,
        }
    }
}

impl Inner {
    fn key(&self, path: &BlobPath) -> String {
        self.prefix.as_ref().map_or_else(
            || path.as_str().to_owned(),
            |prefix| format!("{prefix}/{path}"),
        )
    }

    /// Signs and sends `call`, retrying through [`Http::send`]. The
    /// signature is computed once; retries finish well inside the 15
    /// minutes of clock skew S3 accepts.
    async fn send(&self, call: Call<'_>) -> StoreResult<Response> {
        let uri = match call.key {
            Some(key) => format!("{}/{}", self.bucket_path, sigv4::uri_encode(key, true)),
            None if self.bucket_path.is_empty() => "/".to_owned(),
            None => self.bucket_path.clone(),
        };
        let query = sigv4::canonical_query(call.query);
        let url = if query.is_empty() {
            format!("{}{uri}", self.origin)
        } else {
            format!("{}{uri}?{query}", self.origin)
        };
        let payload_hash = if call.body.is_empty() {
            sigv4::EMPTY_SHA256.to_owned()
        } else {
            sigv4::sha256_hex(&call.body)
        };
        let credentials = self.credentials.get().await?;
        let when = sigv4::AmzDate::at(SystemTime::now());
        let mut headers: Vec<(&str, &str)> = vec![
            ("host", &self.host),
            ("x-amz-content-sha256", &payload_hash),
            ("x-amz-date", &when.stamp),
        ];
        if let Some(token) = &credentials.session_token {
            headers.push(("x-amz-security-token", token.expose()));
        }
        headers.extend(call.headers.iter().map(|(n, v)| (*n, v.as_str())));
        let key = sigv4::Key {
            access_key_id: &credentials.access_key_id,
            secret: credentials.secret_access_key.expose(),
            region: &self.region,
            service: "s3",
        };
        let canonical = sigv4::Canonical {
            method: call.method.as_str(),
            uri: &uri,
            query: &query,
            headers: &headers,
            payload_hash: &payload_hash,
        };
        let auth = sigv4::authorization(&key, &canonical, &when)?;
        self.http
            .send(|| {
                // reqwest derives `Host` from the URL; it matches `self.host`.
                let builder = headers.iter().filter(|(n, _)| *n != "host").fold(
                    self.http.client().request(call.method.clone(), &url),
                    |b, (n, v)| b.header(*n, *v),
                );
                builder.header(AUTHORIZATION, &auth).body(call.body.clone())
            })
            .await
    }

    /// Fetches one `ListObjectsV2` page under `prefix`.
    async fn page(&self, prefix: &str, token: Option<&str>) -> StoreResult<list::Page> {
        let mut query = vec![
            ("list-type", "2"),
            ("prefix", prefix),
            ("encoding-type", "url"),
        ];
        if let Some(token) = token {
            query.push(("continuation-token", token));
        }
        let call = Call {
            method: Method::GET,
            key: None,
            query: &query,
            headers: Vec::new(),
            body: Bytes::new(),
        };
        let response = self.send(call).await?;
        let status = response.status();
        let body = response.text().await.map_err(|e| {
            StoreError::Backend(format!("s3: reading listing: {}", e.without_url()))
        })?;
        if !status.is_success() {
            let excerpt = body.get(..body.len().min(300)).unwrap_or(&body);
            return Err(StoreError::Backend(format!(
                "s3: {status} listing {prefix:?}: {}",
                excerpt.trim()
            )));
        }
        list::parse(&body).map_err(|e| StoreError::Backend(format!("s3: bad listing: {e}")))
    }

    /// Maps a listed key back to a blob path: `None` for keys outside the
    /// prefix, temporary files and keys that are not valid [`BlobPath`]s.
    fn blob_path(&self, key: &str) -> Option<BlobPath> {
        let rel = match &self.prefix {
            Some(prefix) => key.strip_prefix(prefix.as_str())?.strip_prefix('/')?,
            None => key,
        };
        // `BlobPath::new` also rejects the temp prefix, but say so here
        // rather than rely on it.
        if rel
            .split('/')
            .any(|s| s.starts_with(crate::path::TEMP_PREFIX))
        {
            return None;
        }
        BlobPath::new(rel)
            .inspect_err(|e| tracing::debug!(error = %e, "skipping object with an invalid key"))
            .ok()
    }
}

/// Where a listing has got to.
enum Cursor {
    Start,
    Next(String),
    Done,
}

/// Reads a response's version and time. Size comes from `size` or, when
/// `None`, from `Content-Length`. `modified` is `Last-Modified`, else the
/// response `Date` (`PutObject` sends no `Last-Modified`; S3 stamps the
/// object with the request time, which `Date` approximates).
fn meta_from(path: &BlobPath, headers: &HeaderMap, size: Option<u64>) -> StoreResult<BlobMeta> {
    let text = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let etag = text(ETAG)
        .ok_or_else(|| StoreError::Backend(format!("s3: response for {path} has no ETag")))?;
    let size = match size {
        Some(size) => size,
        None => text(CONTENT_LENGTH)
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| StoreError::Backend(format!("s3: response for {path} has no size")))?,
    };
    let modified = text(LAST_MODIFIED)
        .or_else(|| text(DATE))
        .and_then(|v| httpdate::parse_http_date(v).ok())
        .unwrap_or_else(SystemTime::now);
    Ok(BlobMeta {
        path: path.clone(),
        size,
        modified,
        etag: Etag::new(etag),
    })
}

async fn error(path: &BlobPath, response: Response) -> StoreError {
    status_error(BACKEND, path, response).await
}

#[async_trait]
impl BlobStore for S3Store {
    fn backend(&self) -> &'static str {
        BACKEND
    }

    #[instrument(skip(self), fields(backend = "s3", path = %path), err)]
    async fn get(&self, path: &BlobPath) -> StoreResult<Blob> {
        let key = self.key(path);
        let response = self.0.send(Call::object(Method::GET, &key)).await?;
        if !response.status().is_success() {
            return Err(error(path, response).await);
        }
        let headers = response.headers().clone();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| StoreError::Backend(format!("s3: reading {path}: {}", e.without_url())))?;
        let meta = meta_from(path, &headers, Some(bytes.len() as u64))?;
        Ok(Blob { bytes, meta })
    }

    #[instrument(skip(self), fields(backend = "s3", path = %path), err)]
    async fn head(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        let key = self.key(path);
        let response = self.0.send(Call::object(Method::HEAD, &key)).await?;
        if !response.status().is_success() {
            return Err(error(path, response).await);
        }
        meta_from(path, response.headers(), None)
    }

    #[instrument(skip(self, bytes), fields(backend = "s3", path = %path, size = bytes.len()), err)]
    async fn put(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        condition: Precondition,
    ) -> StoreResult<BlobMeta> {
        let key = self.key(path);
        let size = bytes.len() as u64;
        let mut call = Call::object(Method::PUT, &key);
        call.body = bytes;
        let call = match &condition {
            Precondition::None => call,
            Precondition::IfAbsent => call.header("if-none-match", "*"),
            Precondition::IfMatch(etag) => call.header("if-match", etag.as_str()),
        };
        let response = self.0.send(call).await?;
        let status = response.status();
        if status.is_success() {
            return meta_from(path, response.headers(), Some(size));
        }
        let refused = matches!(
            status,
            StatusCode::PRECONDITION_FAILED | StatusCode::CONFLICT
        );
        match condition {
            Precondition::IfAbsent if refused => Err(StoreError::AlreadyExists(path.clone())),
            Precondition::IfMatch(_) if status == StatusCode::NOT_FOUND => {
                Err(StoreError::PreconditionFailed {
                    path: path.clone(),
                    current: None,
                })
            }
            Precondition::IfMatch(_) if refused => Err(self.changed(path).await),
            _ => Err(error(path, response).await),
        }
    }

    /// Deletes with `If-Match` on `DeleteObject` for `IfMatch`, so a stale
    /// delete is refused by the bucket.
    ///
    /// Not every S3 implementation supports that header: `LocalStack` 4
    /// answers `501 NotImplemented` naming `If-Match`. On that answer this
    /// store falls back, for this and every later delete, to a `HEAD` that
    /// compares the etag followed by an unconditional `DELETE`. That
    /// fallback is not atomic: a write landing between the two requests is
    /// deleted, and the watcher reports the removal.
    ///
    /// S3 answers a delete of a missing key with success, so `None` and
    /// `IfAbsent` first `HEAD` the object to report
    /// [`StoreError::NotFound`] / [`StoreError::AlreadyExists`]. If the
    /// object vanishes between the two requests the delete still succeeds.
    #[instrument(skip(self), fields(backend = "s3", path = %path), err)]
    async fn delete(&self, path: &BlobPath, condition: Precondition) -> StoreResult<()> {
        let key = self.key(path);
        let call = Call::object(Method::DELETE, &key);
        let call = match condition {
            Precondition::None => {
                let _ = self.head(path).await?;
                call
            }
            Precondition::IfAbsent => {
                let _ = self.head(path).await?;
                return Err(StoreError::AlreadyExists(path.clone()));
            }
            Precondition::IfMatch(etag) if self.0.no_conditional_delete.load(Ordering::Relaxed) => {
                self.check_then(path, &etag).await?;
                call
            }
            Precondition::IfMatch(etag) => {
                let response = self.0.send(call.header("if-match", etag.as_str())).await?;
                if response.status() != StatusCode::NOT_IMPLEMENTED {
                    return self.deleted(path, response).await;
                }
                tracing::info!(
                    "bucket rejects If-Match on DeleteObject; checking with HEAD instead"
                );
                self.0.no_conditional_delete.store(true, Ordering::Relaxed);
                self.check_then(path, &etag).await?;
                Call::object(Method::DELETE, &key)
            }
        };
        let response = self.0.send(call).await?;
        self.deleted(path, response).await
    }

    /// S3 has no rename, so this copies through the client and never
    /// clobbers: `GET from`, `PUT to` with `If-None-Match: *`, then
    /// `DELETE from` with `If-Match` on the etag that was copied (or the
    /// non-atomic `HEAD` check where the bucket rejects that header; see
    /// [`BlobStore::delete`] on this type).
    ///
    /// If `from` changes or vanishes before the delete, the copy at `to` is
    /// removed again (conditionally on its own etag) and the error is
    /// returned, so a concurrent edit is never lost. If the process dies
    /// or the delete fails otherwise, both blobs remain with the same
    /// content: the watcher reports that as `Duplicated`.
    #[instrument(skip(self), fields(backend = "s3", from = %from, to = %to), err)]
    async fn rename(&self, from: &BlobPath, to: &BlobPath) -> StoreResult<BlobMeta> {
        let source = self.get(from).await?;
        let moved = self.put(to, source.bytes, Precondition::IfAbsent).await?;
        match self
            .delete(from, Precondition::IfMatch(source.meta.etag))
            .await
        {
            Ok(()) => Ok(moved),
            Err(e @ StoreError::PreconditionFailed { .. }) => {
                let undo = self
                    .delete(to, Precondition::IfMatch(moved.etag.clone()))
                    .await;
                if let Err(undo) = undo {
                    tracing::warn!(error = %undo, "could not remove the copy of a changed source");
                }
                Err(e)
            }
            Err(e) => Err(e),
        }
    }

    fn list(&self, dir: Option<&BlobPath>) -> BoxStream<'static, StoreResult<BlobEntry>> {
        let inner = Arc::clone(&self.0);
        let dir = dir.cloned();
        let span =
            tracing::info_span!("list", backend = "s3", dir = ?dir.as_ref().map(BlobPath::as_str));
        // Listing `<key of dir>` without a trailing slash also finds a blob
        // stored at `dir` itself; siblings such as `dir-2/` are filtered out.
        let prefix = match (&dir, &inner.prefix) {
            (Some(dir), _) => inner.key(dir),
            (None, Some(prefix)) => format!("{prefix}/"),
            (None, None) => String::new(),
        };
        futures::stream::unfold(Cursor::Start, move |cursor| {
            let (inner, prefix, dir) = (Arc::clone(&inner), prefix.clone(), dir.clone());
            async move {
                let token = match cursor {
                    Cursor::Start => None,
                    Cursor::Next(token) => Some(token),
                    Cursor::Done => return None,
                };
                let page = match inner.page(&prefix, token.as_deref()).await {
                    Ok(page) => page,
                    Err(e) => return Some((vec![Err(e)], Cursor::Done)),
                };
                let entries = page
                    .objects
                    .into_iter()
                    .flatten()
                    .filter_map(|o| {
                        let path = inner.blob_path(&o.key)?;
                        let inside = dir.as_ref().is_none_or(|d| path.starts_with(d));
                        inside.then_some(Ok(BlobEntry {
                            path,
                            size: o.size,
                            modified: o.modified,
                        }))
                    })
                    .collect();
                Some((entries, page.next.map_or(Cursor::Done, Cursor::Next)))
            }
            .instrument(span.clone())
        })
        .flat_map(futures::stream::iter)
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(endpoint: &str, addressing: Addressing, prefix: Option<&str>) -> S3Config {
        S3Config {
            endpoint: endpoint.into(),
            region: "us-east-1".into(),
            bucket: "notes".into(),
            prefix: prefix.map(Into::into),
            addressing,
            credentials: S3Credentials {
                access_key_id: "AKID".into(),
                secret_access_key: Secret::new("very-secret"),
                session_token: Some(Secret::new("session-secret")),
            }
            .into(),
        }
    }

    fn keys(c: &S3Config) -> &S3Credentials {
        match &c.credentials {
            CredentialSource::Static(keys) => keys,
            other => panic!("expected static keys, got {other:?}"),
        }
    }

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: std::collections::HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| map.get(name).cloned()
    }

    const CREDS: [(&str, &str); 2] = [
        ("AWS_ACCESS_KEY_ID", "AKID"),
        ("AWS_SECRET_ACCESS_KEY", "s3cr3t-value"),
    ];

    #[test]
    fn env_defaults_to_aws_virtual_hosted() -> StoreResult<()> {
        let mut env = CREDS.to_vec();
        env.push(("VELLUM_BLOB_URL", "s3://notes"));
        let c = S3Config::from_vars(vars(&env))?;
        assert_eq!(c.bucket, "notes");
        assert_eq!(c.prefix, None);
        assert_eq!(c.region, "us-east-1");
        assert_eq!(c.endpoint, "https://s3.us-east-1.amazonaws.com");
        assert_eq!(c.addressing, Addressing::VirtualHosted);
        assert_eq!(keys(&c).access_key_id, "AKID");
        assert_eq!(keys(&c).secret_access_key.expose(), "s3cr3t-value");
        assert!(keys(&c).session_token.is_none());
        let s = S3Store::new(c)?;
        assert_eq!(s.0.origin, "https://notes.s3.us-east-1.amazonaws.com");
        Ok(())
    }

    #[test]
    fn env_with_custom_endpoint_prefix_region_and_token() -> StoreResult<()> {
        let mut env = CREDS.to_vec();
        env.extend([
            ("VELLUM_BLOB_URL", "s3://notes/team/one/"),
            ("VELLUM_S3_ENDPOINT", "http://127.0.0.1:9000/"),
            ("VELLUM_S3_REGION", "eu-west-1"),
            ("AWS_SESSION_TOKEN", "tok"),
        ]);
        let c = S3Config::from_vars(vars(&env))?;
        assert_eq!(c.prefix.as_deref(), Some("team/one"));
        assert_eq!(c.region, "eu-west-1");
        assert_eq!(c.endpoint, "http://127.0.0.1:9000");
        assert_eq!(c.addressing, Addressing::Path);
        assert_eq!(
            keys(&c).session_token.as_ref().map(Secret::expose),
            Some("tok")
        );
        let s = S3Store::new(c)?;
        assert_eq!(s.0.bucket_path, "/notes");
        Ok(())
    }

    #[test]
    fn env_errors_name_the_variable_but_not_secrets() {
        let message = |env: &[(&str, &str)]| match S3Config::from_vars(vars(env)) {
            Err(e) => e.to_string(),
            Ok(c) => format!("unexpected success: {c:?}"),
        };
        let url = ("VELLUM_BLOB_URL", "s3://notes");
        let id = ("AWS_ACCESS_KEY_ID", "AKID");
        let secret = ("AWS_SECRET_ACCESS_KEY", "s3cr3t-value");
        assert!(message(&[id, secret]).contains("VELLUM_BLOB_URL is not set"));
        // No keys on a custom endpoint: nothing to fall back to.
        let minio = ("VELLUM_S3_ENDPOINT", "http://127.0.0.1:9000");
        assert!(message(&[url, minio, secret]).contains("no credentials"));
        // No keys on AWS: the instance profile.
        assert_eq!(
            S3Config::from_vars(vars(&[url]))
                .map(|c| c.credentials.kind())
                .ok(),
            Some("instance_profile")
        );
        let missing_secret = message(&[url, id]);
        assert!(missing_secret.contains("AWS_SECRET_ACCESS_KEY is not"));
        let empty = message(&[url, id, ("AWS_SECRET_ACCESS_KEY", "")]);
        assert!(empty.contains("AWS_SECRET_ACCESS_KEY is not"));
        for bad in ["https://notes", "s3://", "s3:///prefix"] {
            let got = message(&[("VELLUM_BLOB_URL", bad), id, secret]);
            assert!(got.contains("VELLUM_BLOB_URL"), "{bad}: {got}");
            assert!(!got.contains("s3cr3t-value"), "{got}");
        }
        let ok = S3Config::from_vars(vars(&[url, id, secret]));
        assert!(ok.is_ok_and(|c| !format!("{c:?}").contains("s3cr3t-value")));
    }

    #[test]
    fn virtual_hosted_and_path_style_urls() -> StoreResult<()> {
        let s = S3Store::new(config(
            "https://s3.eu-west-1.amazonaws.com",
            Addressing::VirtualHosted,
            None,
        ))?;
        assert_eq!(s.0.origin, "https://notes.s3.eu-west-1.amazonaws.com");
        assert_eq!(s.0.host, "notes.s3.eu-west-1.amazonaws.com");
        assert_eq!(s.0.bucket_path, "");
        let s = S3Store::new(config("http://127.0.0.1:4566", Addressing::Path, None))?;
        assert_eq!(s.0.origin, "http://127.0.0.1:4566");
        assert_eq!(s.0.host, "127.0.0.1:4566");
        assert_eq!(s.0.bucket_path, "/notes");
        Ok(())
    }

    #[test]
    fn keys_carry_the_prefix() -> StoreResult<()> {
        let s = S3Store::new(config("http://h", Addressing::Path, Some("team/a")))?;
        let path = BlobPath::new("x/Notes – ü.md")?;
        assert_eq!(s.key(&path), "team/a/x/Notes – ü.md");
        assert_eq!(s.0.blob_path("team/a/x/Notes – ü.md"), Some(path));
        assert_eq!(s.0.blob_path("team/ab/x.md"), None);
        assert_eq!(s.0.blob_path("team/a/.vellum-tmp-1"), None);
        assert_eq!(s.0.blob_path("team/a/a//b"), None);
        Ok(())
    }

    #[test]
    fn rejects_bad_configuration() {
        for endpoint in ["ftp://h", "http://h/path", "http://user:pw@h", "not a url"] {
            let got = S3Store::new(config(endpoint, Addressing::Path, None));
            assert!(got.is_err(), "{endpoint}");
        }
        let mut bad_bucket = config("http://h", Addressing::Path, None);
        bad_bucket.bucket = "Bad/Bucket".into();
        assert!(S3Store::new(bad_bucket).is_err());
        let bad_prefix = config("http://h", Addressing::Path, Some("../up"));
        assert!(matches!(
            S3Store::new(bad_prefix),
            Err(StoreError::InvalidPath { .. })
        ));
    }

    #[test]
    fn debug_output_hides_secrets() -> StoreResult<()> {
        let s = S3Store::new(config("http://h", Addressing::Path, None))?;
        let debug = format!("{s:?}");
        assert!(!debug.contains("very-secret") && !debug.contains("session-secret"));
        Ok(())
    }

    #[test]
    fn meta_prefers_last_modified_then_date() -> StoreResult<()> {
        let path = BlobPath::new("a.md")?;
        let mut headers = HeaderMap::new();
        let _ = headers.insert(
            ETAG,
            "\"abc\""
                .parse()
                .map_err(|_| StoreError::Backend(String::new()))?,
        );
        let _ = headers.insert(
            DATE,
            "Thu, 01 Jan 1970 00:00:10 GMT"
                .parse()
                .map_err(|_| StoreError::Backend(String::new()))?,
        );
        let meta = meta_from(&path, &headers, Some(3))?;
        assert_eq!(meta.etag.as_str(), "\"abc\"");
        assert_eq!(
            meta.modified,
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(10)
        );
        let _ = headers.insert(
            LAST_MODIFIED,
            "Thu, 01 Jan 1970 00:00:05 GMT"
                .parse()
                .map_err(|_| StoreError::Backend(String::new()))?,
        );
        let _ = headers.insert(
            CONTENT_LENGTH,
            "7".parse()
                .map_err(|_| StoreError::Backend(String::new()))?,
        );
        let meta = meta_from(&path, &headers, None)?;
        assert_eq!(meta.size, 7);
        assert_eq!(
            meta.modified,
            SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(5)
        );
        Ok(())
    }
}
