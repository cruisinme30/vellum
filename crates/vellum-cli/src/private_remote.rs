// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Private spaces on a Vellum server, from this device (ADR 0043, D185).
//!
//! [`E2eClient`] calls the server's `e2e` API with the device's bearer
//! token. On top of it:
//!
//! - [`enrol`] adds this device's public key to a space;
//! - [`reseal`] brings every page this device can open in line with the
//!   space's current keys, after someone enrolled or revoked a device;
//! - [`RemoteSpaces`] is a read-only [`BlobStore`] over the API, so the
//!   private agent (`vellum mcp --private --server …`) opens a space
//!   held by a server exactly as it opens a local folder.
//!
//! Only ciphertext and public keys cross the network. Pages are opened and
//! sealed here, in memory.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use futures::stream::{self, BoxStream, StreamExt as _};
use serde::de::DeserializeOwned;
use vellum_crypto::recipients::{
    RecipientPublic, is_recipient_envelope, recipients_of, seal_for,
};
use vellum_e2e::{
    DeviceKeyInfo, ObjectInfo, ObjectList, SealedObject, SpaceKeys, Stored,
};
use vellum_kernel::SpaceId;
use vellum_private::DeviceIdentity;
use vellum_store::{
    Blob, BlobEntry, BlobMeta, BlobPath, BlobStore, Etag, Precondition,
    StoreError, StoreResult,
};

use crate::pins::Pins;

/// How long one API call may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Variable holding the server's URL.
pub const SERVER_VAR: &str = "VELLUM_SERVER";

/// Variable holding the bearer token (an API token, or the desktop app's
/// access token).
pub const TOKEN_VAR: &str = "VELLUM_TOKEN";

/// Why a call to the server failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiFailure {
    /// The HTTP status, 0 when the server was not reached.
    pub status: u16,
    /// The server's error code, such as `conflict`.
    pub code: String,
    /// What went wrong.
    pub message: String,
}

impl std::fmt::Display for ApiFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.status == 0 {
            write!(f, "cannot reach the server: {}", self.message)
        } else {
            write!(f, "{} ({}): {}", self.code, self.status, self.message)
        }
    }
}

impl std::error::Error for ApiFailure {}

fn unreachable(error: reqwest::Error) -> ApiFailure {
    ApiFailure {
        status: 0,
        code: "unavailable".into(),
        message: error.without_url().to_string(),
    }
}

/// An object as fetched.
#[derive(Debug, Clone)]
pub struct Fetched {
    /// Its bytes: an envelope when `sealed`.
    pub bytes: Vec<u8>,
    /// Its version.
    pub etag: String,
    /// False for a readable object.
    pub sealed: bool,
}

/// The server's `e2e` API, as one signed-in device.
#[derive(Clone)]
pub struct E2eClient {
    http: reqwest::Client,
    base: url::Url,
    token: String,
}

impl std::fmt::Debug for E2eClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("E2eClient")
            .field("base", &self.base.as_str())
            .finish_non_exhaustive()
    }
}

impl E2eClient {
    /// A client for the server at `server` (such as
    /// `https://vellum.northstar.dev`) with a bearer `token`.
    ///
    /// # Errors
    ///
    /// The URL is not http(s), or plain http to a host other than this
    /// machine, or the token is empty.
    pub fn new(server: &str, token: &str) -> Result<Self, String> {
        let base = url::Url::parse(server.trim())
            .map_err(|e| format!("--server: {e}"))?;
        let local = matches!(
            base.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        );
        match base.scheme() {
            "https" => {}
            "http" if local => {}
            _ => return Err("--server must be an https:// URL".into()),
        }
        if token.trim().is_empty() {
            return Err(format!(
                "set {TOKEN_VAR} to an API token for this server"
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("vellum/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("cannot build the HTTP client: {e}"))?;
        Ok(Self {
            http,
            base,
            token: token.trim().to_owned(),
        })
    }

    /// The server's origin, as pins are keyed.
    #[must_use]
    pub fn server(&self) -> String {
        self.base.origin().ascii_serialization()
    }

    fn url(&self, space: SpaceId, tail: &str) -> Result<url::Url, ApiFailure> {
        self.base
            .join(&format!("/api/v1/spaces/{space}/e2e/{tail}"))
            .map_err(|e| ApiFailure {
                status: 0,
                code: "invalid_input".into(),
                message: e.to_string(),
            })
    }

    async fn send<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, ApiFailure> {
        let response = request
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(unreachable)?;
        let status = response.status();
        if status.is_success() {
            return response.json().await.map_err(unreachable);
        }
        let body: serde_json::Value = response.json().await.unwrap_or_default();
        Err(ApiFailure {
            status: status.as_u16(),
            code: body
                .get("code")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("error")
                .to_owned(),
            message: body
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_else(|| {
                    status.canonical_reason().unwrap_or("failed")
                })
                .to_owned(),
        })
    }

    /// The space's key list.
    ///
    /// # Errors
    ///
    /// The server refused or could not be reached.
    pub async fn keys(&self, space: SpaceId) -> Result<SpaceKeys, ApiFailure> {
        self.send(self.http.get(self.url(space, "keys")?)).await
    }

    /// Enrols `public_key` (hex) under `label`.
    ///
    /// # Errors
    ///
    /// The server refused or could not be reached.
    pub async fn enrol(
        &self,
        space: SpaceId,
        public_key: &str,
        label: &str,
    ) -> Result<DeviceKeyInfo, ApiFailure> {
        let body =
            serde_json::json!({ "public_key": public_key, "label": label });
        self.send(self.http.post(self.url(space, "keys")?).json(&body))
            .await
    }

    /// Every object in the space, following pages.
    ///
    /// # Errors
    ///
    /// The server refused or could not be reached.
    pub async fn objects(
        &self,
        space: SpaceId,
    ) -> Result<Vec<ObjectInfo>, ApiFailure> {
        let mut all = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let mut url = self.url(space, "objects")?;
            if let Some(after) = &after {
                let _ = url.query_pairs_mut().append_pair("after", after);
            }
            let page: ObjectList = self.send(self.http.get(url)).await?;
            all.extend(page.objects);
            match page.next {
                Some(next) => after = Some(next),
                None => return Ok(all),
            }
        }
    }

    /// One object's bytes and version.
    ///
    /// # Errors
    ///
    /// The server refused, could not be reached, or sent bad base64.
    pub async fn get(
        &self,
        space: SpaceId,
        path: &str,
    ) -> Result<(Vec<u8>, String), ApiFailure> {
        let object = self.fetch(space, path, false).await?;
        Ok((object.bytes, object.etag))
    }

    /// One object, readable ones included (needs edit access), so this
    /// device can seal pages left readable before the space turned
    /// private.
    ///
    /// # Errors
    ///
    /// The server refused, could not be reached, or sent bad base64.
    pub async fn fetch(
        &self,
        space: SpaceId,
        path: &str,
        plaintext: bool,
    ) -> Result<Fetched, ApiFailure> {
        let mut url = self.url(space, "object")?;
        let _ = url.query_pairs_mut().append_pair("path", path);
        if plaintext {
            let _ = url.query_pairs_mut().append_pair("plaintext", "true");
        }
        let object: SealedObject = self.send(self.http.get(url)).await?;
        let bytes =
            STANDARD.decode(object.envelope).map_err(|_| ApiFailure {
                status: 0,
                code: "internal".into(),
                message: "the server sent bad base64".into(),
            })?;
        Ok(Fetched {
            bytes,
            etag: object.etag,
            sealed: object.sealed,
        })
    }

    /// Stores sealed bytes at `path`, replacing version `if_match`.
    ///
    /// # Errors
    ///
    /// The server refused (`conflict` when the object changed or the
    /// keys did) or could not be reached.
    pub async fn put(
        &self,
        space: SpaceId,
        path: &str,
        envelope: &[u8],
        if_match: Option<&str>,
    ) -> Result<Stored, ApiFailure> {
        let body = serde_json::json!({
            "path": path,
            "envelope": STANDARD.encode(envelope),
            "if_match": if_match,
        });
        self.send(self.http.put(self.url(space, "object")?).json(&body))
            .await
    }
}

/// Enrols this device in `space` and returns what the server recorded.
///
/// # Errors
///
/// The server refused (for example the key is already enrolled) or could
/// not be reached.
pub async fn enrol(
    client: &E2eClient,
    identity: &DeviceIdentity,
    space: SpaceId,
    label: &str,
    pins: &Pins,
) -> Result<DeviceKeyInfo, String> {
    let enrolled = client
        .enrol(space, &identity.public_hex(), label)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(
        space = %space,
        fingerprint = %enrolled.fingerprint,
        "device enrolled"
    );
    // A device joining a space trusts the keys it finds there, its own
    // included; any key that appears later needs trusting by hand.
    let server = client.server();
    let mut trusted = pins.load(&server, space)?.unwrap_or_default();
    if trusted.is_empty() {
        let keys = client.keys(space).await.map_err(|e| e.to_string())?;
        trusted.extend(keys.keys.into_iter().map(|k| k.fingerprint));
    }
    let _ = trusted.insert(enrolled.fingerprint.clone());
    pins.save(&server, space, &trusted)?;
    Ok(enrolled)
}

/// The start of the error naming device keys this device does not trust
/// yet; the desktop app reads the fingerprints after it.
pub const UNTRUSTED: &str = "untrusted device keys: ";

/// What a reseal pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ResealReport {
    /// Pages already sealed to exactly the current keys.
    pub current: usize,
    /// Pages resealed and uploaded.
    pub resealed: usize,
    /// Readable pages, left from before the space turned private, now
    /// sealed and uploaded.
    pub converted: usize,
    /// True when this was the first reseal of the space on this device,
    /// so its key list was trusted as found.
    pub first_use: bool,
    /// Pages this device cannot open; another member's device reseals them.
    pub not_for_this_device: usize,
    /// Pages that changed on the server during the pass; run it again.
    pub changed: usize,
}

/// Reseals every page of `space` that this device can open to exactly
/// the space's current keys: new devices are added and revoked ones
/// dropped. Each page gets a fresh file key, so a revoked device cannot
/// read later versions even if it kept the old one. Pages still readable
/// from before the space turned private are sealed too (D209).
///
/// Nothing is sealed to a key this device has not trusted before (D210):
/// the pass stops with an [`UNTRUSTED`] error naming the new keys unless
/// their fingerprints are in `trust`.
///
/// # Errors
///
/// The space has no keys, a key is untrusted or malformed, or the server
/// refused.
pub async fn reseal(
    client: &E2eClient,
    identity: &DeviceIdentity,
    space: SpaceId,
    pins: &Pins,
    trust: &[String],
) -> Result<ResealReport, String> {
    let keys = client.keys(space).await.map_err(|e| e.to_string())?;
    if keys.keys.is_empty() {
        return Err(
            "this space has no device keys; enrol a device first".into()
        );
    }
    let server = client.server();
    let pinned = pins.load(&server, space)?;
    let mut report = ResealReport {
        first_use: pinned.is_none(),
        ..ResealReport::default()
    };
    if let Some(pinned) = &pinned {
        check_trust(&keys.keys, pinned, trust)?;
    }
    let recipients = recipients_from(&keys.keys)?;
    let mut wanted: Vec<[u8; 16]> = recipients
        .iter()
        .map(|r| r.fingerprint().to_bytes())
        .collect();
    wanted.sort_unstable();
    for object in client.objects(space).await.map_err(|e| e.to_string())? {
        if vellum_e2e::structural(&object.path) {
            continue;
        }
        let fetched = match client.fetch(space, &object.path, true).await {
            Ok(found) => found,
            // Deleted since the listing.
            Err(e) if e.status == 404 => continue,
            Err(e) => return Err(e.to_string()),
        };
        let plain = if fetched.sealed && is_recipient_envelope(&fetched.bytes) {
            let mut have: Vec<[u8; 16]> = recipients_of(&fetched.bytes)
                .map_err(|e| format!("{}: {e}", object.path))?
                .iter()
                .map(vellum_crypto::recipients::Fingerprint::to_bytes)
                .collect();
            have.sort_unstable();
            have.dedup();
            if have == wanted {
                report.current += 1;
                continue;
            }
            let Ok(plain) = identity.open(&fetched.bytes) else {
                report.not_for_this_device += 1;
                continue;
            };
            plain
        } else {
            fetched.bytes
        };
        let was_sealed = fetched.sealed;
        let sealed = seal_for(&recipients, &plain)
            .map_err(|e| format!("{}: {e}", object.path))?;
        match client
            .put(space, &object.path, &sealed, Some(&fetched.etag))
            .await
        {
            Ok(_) if was_sealed => report.resealed += 1,
            Ok(_) => report.converted += 1,
            Err(e) if e.status == 409 => report.changed += 1,
            Err(e) => return Err(e.to_string()),
        }
    }
    let trusted: BTreeSet<String> =
        keys.keys.into_iter().map(|k| k.fingerprint).collect();
    pins.save(&server, space, &trusted)?;
    tracing::info!(
        space = %space,
        current = report.current,
        resealed = report.resealed,
        converted = report.converted,
        not_for_this_device = report.not_for_this_device,
        changed = report.changed,
        "private space resealed"
    );
    Ok(report)
}

/// Refuses keys that are neither pinned nor in `trust` (D210).
fn check_trust(
    keys: &[DeviceKeyInfo],
    pinned: &BTreeSet<String>,
    trust: &[String],
) -> Result<(), String> {
    let untrusted: Vec<&DeviceKeyInfo> = keys
        .iter()
        .filter(|k| {
            !pinned.contains(&k.fingerprint) && !trust.contains(&k.fingerprint)
        })
        .collect();
    if !untrusted.is_empty() {
        let fingerprints: Vec<&str> =
            untrusted.iter().map(|k| k.fingerprint.as_str()).collect();
        let labels: Vec<&str> =
            untrusted.iter().map(|k| k.label.as_str()).collect();
        return Err(format!(
            "{UNTRUSTED}{} ({}); compare each fingerprint with its owner, then \
                run again with --trust <fingerprint>",
            fingerprints.join(","),
            labels.join(", ")
        ));
    }
    Ok(())
}

/// The space's keys as recipients, each checked against its fingerprint.
fn recipients_from(
    keys: &[DeviceKeyInfo],
) -> Result<Vec<RecipientPublic>, String> {
    keys.iter()
        .map(|k| {
            let bytes: [u8; 32] = hex::decode(&k.public_key)
                .ok()
                .and_then(|b| b.try_into().ok())
                .ok_or_else(|| {
                    format!("the server sent a malformed key for {}", k.label)
                })?;
            let public = RecipientPublic::from_bytes(bytes);
            if hex::encode(public.fingerprint().to_bytes()) != k.fingerprint {
                return Err(format!(
                    "the server sent a mismatched fingerprint for {}",
                    k.label
                ));
            }
            Ok(public)
        })
        .collect::<Result<Vec<_>, String>>()
}

/// Private spaces on a server as a read-only [`BlobStore`]: the space
/// with ID `S` appears as the folder `S`, so `PrivateSpace::open` and the
/// agent read it like a local one.
#[derive(Debug, Clone)]
pub struct RemoteSpaces {
    client: E2eClient,
}

impl RemoteSpaces {
    /// Reads spaces through `client`.
    #[must_use]
    pub const fn new(client: E2eClient) -> Self {
        Self { client }
    }
}

fn split(path: &BlobPath) -> StoreResult<(SpaceId, String)> {
    let invalid = || StoreError::InvalidPath {
        path: path.to_string(),
        reason: "not inside a remote private space",
    };
    let (space, rest) = path.as_str().split_once('/').ok_or_else(invalid)?;
    let space: SpaceId = space.parse().map_err(|_| invalid())?;
    Ok((space, rest.to_owned()))
}

fn store_failure(path: &BlobPath, failure: &ApiFailure) -> StoreError {
    match failure.status {
        404 => StoreError::NotFound(path.clone()),
        _ => StoreError::Backend(failure.to_string()),
    }
}

fn read_only() -> StoreError {
    StoreError::Backend("remote private spaces are read-only here".into())
}

fn system_time(at: vellum_kernel::Timestamp) -> SystemTime {
    u64::try_from(at.unix_ms())
        .ok()
        .and_then(|ms| {
            SystemTime::UNIX_EPOCH.checked_add(Duration::from_millis(ms))
        })
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

#[async_trait]
impl BlobStore for RemoteSpaces {
    fn backend(&self) -> &'static str {
        "vellum-e2e"
    }

    async fn get(&self, path: &BlobPath) -> StoreResult<Blob> {
        let (space, rel) = split(path)?;
        let (bytes, etag) = self
            .client
            .get(space, &rel)
            .await
            .map_err(|e| store_failure(path, &e))?;
        Ok(Blob {
            meta: BlobMeta {
                path: path.clone(),
                size: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                modified: SystemTime::now(),
                etag: Etag::new(etag),
            },
            bytes: Bytes::from(bytes),
        })
    }

    async fn head(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        Ok(self.get(path).await?.meta)
    }

    async fn put(
        &self,
        _: &BlobPath,
        _: Bytes,
        _: Precondition,
    ) -> StoreResult<BlobMeta> {
        Err(read_only())
    }

    async fn delete(&self, _: &BlobPath, _: Precondition) -> StoreResult<()> {
        Err(read_only())
    }

    async fn rename(
        &self,
        _: &BlobPath,
        _: &BlobPath,
    ) -> StoreResult<BlobMeta> {
        Err(read_only())
    }

    fn list(
        &self,
        dir: Option<&BlobPath>,
    ) -> BoxStream<'static, StoreResult<BlobEntry>> {
        let client = self.client.clone();
        let dir = dir.cloned();
        stream::once(async move {
            let Some(dir) = dir else {
                return vec![Err(StoreError::Backend(
                    "list a remote private space by its ID".into(),
                ))];
            };
            let (space, prefix) = match dir.as_str().split_once('/') {
                Some((space, prefix)) => {
                    (space.to_owned(), format!("{prefix}/"))
                }
                None => (dir.as_str().to_owned(), String::new()),
            };
            let Ok(space) = space.parse::<SpaceId>() else {
                return vec![Err(StoreError::NotFound(dir))];
            };
            match client.objects(space).await {
                Ok(objects) => objects
                    .into_iter()
                    .filter(|o| o.path.starts_with(&prefix))
                    .map(|o| {
                        Ok(BlobEntry {
                            path: BlobPath::new(format!("{space}/{}", o.path))?,
                            size: o.size,
                            modified: system_time(o.modified),
                        })
                    })
                    .collect(),
                Err(e) => vec![Err(store_failure(&dir, &e))],
            }
        })
        .flat_map(stream::iter)
        .boxed()
    }
}
