// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The application service: key lists and ciphertext.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use futures::TryStreamExt as _;
use vellum_crypto::recipients::{is_recipient_envelope, recipients_of};
use vellum_kernel::{Error, ErrorKind, SpaceId, Timestamp, UserId};
use vellum_store::layout::{FOLDER_META, SYSTEM_DIR};
use vellum_store::{BlobPath, BlobStore, Etag, Precondition, StoreError};

use crate::api::{
    DeleteObject, Deleted, DeviceKeyInfo, EnrolDevice, GetObject, ListObjects, MAX_ENVELOPE_BYTES,
    MAX_LABEL_CHARS, ObjectInfo, ObjectList, PutObject, RevokeDevice, Revoked, SealedObject,
    SpaceKeys, Stored,
};
use crate::keys::{Entry, KeyFile, keys_path, parse_key};
use crate::ports::{Need, SpaceAccess};

/// Attempts at a key-list change before giving up on a busy space.
const KEY_WRITE_ATTEMPTS: usize = 5;

/// Largest page of objects.
const MAX_PAGE: usize = 1000;

/// The service: key lists and ciphertext for E2E private spaces.
pub struct E2e {
    store: Arc<dyn BlobStore>,
    access: Arc<dyn SpaceAccess>,
}

impl fmt::Debug for E2e {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("E2e")
            .field("store", &self.store.backend())
            .field("access", &self.access)
            .finish()
    }
}

/// A key list as read, with the version it was read at.
struct Loaded {
    file: KeyFile,
    etag: Option<Etag>,
}

impl E2e {
    /// The service over `store` (the same store the drive uses) and
    /// `access`.
    #[must_use]
    pub fn new(store: Arc<dyn BlobStore>, access: Arc<dyn SpaceAccess>) -> Self {
        Self { store, access }
    }

    /// The blob store, for readiness checks.
    #[must_use]
    pub fn store(&self) -> &dyn BlobStore {
        self.store.as_ref()
    }

    /// The space's folder, once `user` is known to hold `need`. A space the
    /// caller cannot see is reported as not found.
    async fn space_root(
        &self,
        user: UserId,
        space: SpaceId,
        need: Need,
    ) -> Result<BlobPath, Error> {
        let not_found = || Error::new(ErrorKind::NotFound, "no such space");
        let root = self.access.root(space).await?.ok_or_else(not_found)?;
        if !self.access.allowed(user, space, Need::View).await? {
            return Err(not_found());
        }
        if need > Need::View && !self.access.allowed(user, space, need).await? {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                match need {
                    Need::Manage => "only a manager of the space can do this",
                    _ => "you cannot change this space",
                },
            ));
        }
        Ok(root)
    }

    async fn load(&self, space: SpaceId) -> Result<Loaded, Error> {
        match self.store.get(&keys_path(space)?).await {
            Ok(blob) => {
                let text = std::str::from_utf8(&blob.bytes).map_err(|e| {
                    Error::new(ErrorKind::Internal, "the space's key list is not text")
                        .with_source(e)
                })?;
                Ok(Loaded {
                    file: KeyFile::parse(text)?,
                    etag: Some(blob.meta.etag),
                })
            }
            Err(StoreError::NotFound(_)) => Ok(Loaded {
                file: KeyFile::empty(),
                etag: None,
            }),
            Err(e) => Err(store_error(e)),
        }
    }

    /// Writes `file` if the list is still at `etag`. `Ok(None)` when
    /// another writer got there first.
    async fn save(
        &self,
        space: SpaceId,
        file: &KeyFile,
        etag: Option<Etag>,
    ) -> Result<Option<Etag>, Error> {
        let condition = etag.map_or(Precondition::IfAbsent, Precondition::IfMatch);
        let path = keys_path(space)?;
        let bytes = Bytes::from(file.render()?);
        match self.store.put(&path, bytes, condition).await {
            Ok(meta) => Ok(Some(meta.etag)),
            Err(StoreError::PreconditionFailed { .. } | StoreError::AlreadyExists(_)) => Ok(None),
            Err(e) => Err(store_error(e)),
        }
    }

    fn keys_of(space: SpaceId, loaded: &Loaded, caller: UserId) -> Result<SpaceKeys, Error> {
        Ok(SpaceKeys {
            space_id: space,
            version: loaded
                .etag
                .as_ref()
                .map(|e| e.as_str().to_owned())
                .unwrap_or_default(),
            keys: loaded
                .file
                .keys
                .iter()
                .map(|k| k.info(caller))
                .collect::<Result<_, _>>()?,
        })
    }

    /// The space's key list.
    ///
    /// # Errors
    ///
    /// `not_found` for a space the caller cannot see.
    #[tracing::instrument(name = "e2e.keys", skip(self), err(level = "debug"))]
    pub async fn keys(&self, user: UserId, space: SpaceId) -> Result<SpaceKeys, Error> {
        let _root = self.space_root(user, space, Need::View).await?;
        let loaded = self.load(space).await?;
        Self::keys_of(space, &loaded, user)
    }

    /// Enrols the caller's device. The first key makes the space private,
    /// so it needs a manager; after that any member enrols their own
    /// devices.
    ///
    /// # Errors
    ///
    /// `invalid_input` for a bad key or label, `permission_denied` for a
    /// first key from a non-manager, `conflict` for a duplicate or a full
    /// list.
    #[tracing::instrument(name = "e2e.enrol", skip(self, request), fields(space = %request.space_id), err(level = "debug"))]
    pub async fn enrol(&self, user: UserId, request: EnrolDevice) -> Result<DeviceKeyInfo, Error> {
        let space = request.space_id;
        let key = parse_key(&request.public_key)?;
        let label = clean_label(&request.label)?;
        let _root = self.space_root(user, space, Need::View).await?;
        for _ in 0..KEY_WRITE_ATTEMPTS {
            let mut loaded = self.load(space).await?;
            if loaded.file.keys.is_empty() {
                let _root = self.space_root(user, space, Need::Manage).await?;
            }
            let mut registry = loaded.file.registry(space)?;
            registry.enroll(space, user, key)?;
            let entry = Entry {
                user,
                key: hex::encode(key.to_bytes()),
                label: label.clone(),
                added: Timestamp::now(),
            };
            let info = entry.info(user)?;
            loaded.file.keys.push(entry);
            if self.save(space, &loaded.file, loaded.etag).await?.is_some() {
                tracing::info!(
                    space = %space,
                    user = %user,
                    fingerprint = %info.fingerprint,
                    monotonic_counter.vellum.e2e.devices_enrolled = 1_u64,
                    "device enrolled in a private space"
                );
                return Ok(info);
            }
        }
        Err(busy())
    }

    /// Revokes one device key: the caller's own, or anyone's for a
    /// manager. The answer lists what members' devices must strip from
    /// existing envelopes.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown key, `permission_denied` for another
    /// member's key without manage access, `conflict` for the last key.
    #[tracing::instrument(name = "e2e.revoke", skip(self, request), fields(space = %request.space_id), err(level = "debug"))]
    pub async fn revoke(&self, user: UserId, request: RevokeDevice) -> Result<Revoked, Error> {
        let space = request.space_id;
        let wanted = request.fingerprint.trim().to_ascii_lowercase();
        let _root = self.space_root(user, space, Need::View).await?;
        for _ in 0..KEY_WRITE_ATTEMPTS {
            let mut loaded = self.load(space).await?;
            let Some(index) = loaded
                .file
                .keys
                .iter()
                .position(|e| e.info(user).is_ok_and(|i| i.fingerprint == wanted))
            else {
                return Err(Error::new(ErrorKind::NotFound, "no such device key"));
            };
            let mut registry = loaded.file.registry(space)?;
            let entry = loaded.file.keys.remove(index);
            if entry.user != user {
                let _root = self.space_root(user, space, Need::Manage).await?;
            }
            let _removed = registry.revoke_device(space, entry.user, entry.device_key()?)?;
            if self.save(space, &loaded.file, loaded.etag).await?.is_some() {
                tracing::info!(
                    space = %space,
                    user = %user,
                    owner = %entry.user,
                    fingerprint = %wanted,
                    monotonic_counter.vellum.e2e.devices_revoked = 1_u64,
                    "device revoked from a private space"
                );
                return Ok(Revoked {
                    space_id: space,
                    removed: vec![entry.info(user)?],
                });
            }
        }
        Err(busy())
    }

    /// A page of the space's objects, in path order.
    ///
    /// # Errors
    ///
    /// `not_found` for a space the caller cannot see; store failures.
    #[tracing::instrument(name = "e2e.objects", skip(self, request), fields(space = %request.space_id), err(level = "debug"))]
    pub async fn objects(&self, user: UserId, request: ListObjects) -> Result<ObjectList, Error> {
        let root = self.space_root(user, request.space_id, Need::View).await?;
        let limit = request
            .limit
            .and_then(|l| usize::try_from(l).ok())
            .unwrap_or(MAX_PAGE)
            .clamp(1, MAX_PAGE);
        let mut objects: Vec<ObjectInfo> = self
            .store
            .list(Some(&root))
            .map_err(store_error)
            .try_filter_map(|entry| {
                let path = relative(&root, &entry.path);
                async move {
                    Ok(path.map(|path| ObjectInfo {
                        path,
                        size: entry.size,
                        modified: modified(entry.modified),
                    }))
                }
            })
            .try_collect()
            .await?;
        objects.sort_by(|a, b| a.path.cmp(&b.path));
        if let Some(after) = &request.after {
            objects.retain(|o| o.path.as_str() > after.as_str());
        }
        let more = objects.len() > limit;
        objects.truncate(limit);
        let next = more
            .then(|| objects.last().map(|o| o.path.clone()))
            .flatten();
        Ok(ObjectList { objects, next })
    }

    /// One object's ciphertext. Only recipient envelopes and the space's
    /// own folder manifest are served.
    ///
    /// # Errors
    ///
    /// `not_found` for a missing object, `conflict` for an object that is
    /// not end-to-end encrypted.
    #[tracing::instrument(name = "e2e.get", skip(self, request), fields(space = %request.space_id), err(level = "debug"))]
    pub async fn get(&self, user: UserId, request: GetObject) -> Result<SealedObject, Error> {
        let root = self.space_root(user, request.space_id, Need::View).await?;
        let (path, rel) = object_path(&root, &request.path)?;
        let blob = self.store.get(&path).await.map_err(store_error)?;
        if rel != FOLDER_META && !is_recipient_envelope(&blob.bytes) {
            return Err(Error::new(
                ErrorKind::Conflict,
                "this object is not end-to-end encrypted; open it through the drive",
            ));
        }
        Ok(SealedObject {
            path: rel,
            etag: blob.meta.etag.as_str().to_owned(),
            envelope: STANDARD.encode(&blob.bytes),
        })
    }

    /// Stores a page the caller's device sealed. The envelope must be
    /// sealed to exactly the space's current keys.
    ///
    /// # Errors
    ///
    /// `invalid_input` for bad base64, a non-envelope or an oversized one;
    /// `conflict` when the recipients do not match the key list, the space
    /// is not private, or the object changed since `if_match`.
    #[tracing::instrument(name = "e2e.put", skip(self, request), fields(space = %request.space_id), err(level = "debug"))]
    pub async fn put(&self, user: UserId, request: PutObject) -> Result<Stored, Error> {
        let space = request.space_id;
        let root = self.space_root(user, space, Need::Edit).await?;
        let (path, rel) = object_path(&root, &request.path)?;
        if request.envelope.len() > MAX_ENVELOPE_BYTES.div_ceil(3) * 4 {
            return Err(too_large());
        }
        let bytes = STANDARD
            .decode(request.envelope.trim())
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "envelope is not base64"))?;
        if bytes.len() > MAX_ENVELOPE_BYTES {
            return Err(too_large());
        }
        if !is_recipient_envelope(&bytes) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "only recipient envelopes can be stored in a private space",
            ));
        }
        let mut sealed_to = recipients_of(&bytes)
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "the envelope is damaged"))?
            .iter()
            .map(vellum_crypto::recipients::Fingerprint::to_bytes)
            .collect::<Vec<_>>();
        sealed_to.sort_unstable();
        sealed_to.dedup();
        let expected = self.load(space).await?.file.fingerprints()?;
        if expected.is_empty() {
            return Err(Error::new(
                ErrorKind::Conflict,
                "this space is not private: enrol a device first",
            ));
        }
        if sealed_to != expected {
            let have: BTreeSet<_> = sealed_to.iter().collect();
            let want: BTreeSet<_> = expected.iter().collect();
            tracing::info!(
                space = %space,
                missing = want.difference(&have).count(),
                extra = have.difference(&want).count(),
                "refused an envelope sealed to other keys than the space's"
            );
            return Err(Error::new(
                ErrorKind::Conflict,
                "the envelope is not sealed to the space's current keys; fetch them and reseal",
            ));
        }
        let condition = request.if_match.map_or(Precondition::IfAbsent, |e| {
            Precondition::IfMatch(Etag::new(e))
        });
        let meta = self
            .store
            .put(&path, Bytes::from(bytes), condition)
            .await
            .map_err(store_error)?;
        tracing::info!(space = %space, user = %user, monotonic_counter.vellum.e2e.objects_stored = 1_u64, "sealed object stored");
        Ok(Stored {
            path: rel,
            etag: meta.etag.as_str().to_owned(),
        })
    }

    /// Deletes an object at the given version.
    ///
    /// # Errors
    ///
    /// `not_found` for a missing object; `conflict` when it changed.
    #[tracing::instrument(name = "e2e.delete", skip(self, request), fields(space = %request.space_id), err(level = "debug"))]
    pub async fn delete(&self, user: UserId, request: DeleteObject) -> Result<Deleted, Error> {
        let root = self.space_root(user, request.space_id, Need::Edit).await?;
        let (path, rel) = object_path(&root, &request.path)?;
        let current = self.store.get(&path).await.map_err(store_error)?;
        if !is_recipient_envelope(&current.bytes) {
            return Err(Error::new(
                ErrorKind::Conflict,
                "this object is not end-to-end encrypted; delete it through the drive",
            ));
        }
        self.store
            .delete(&path, Precondition::IfMatch(Etag::new(request.if_match)))
            .await
            .map_err(store_error)?;
        Ok(Deleted { path: rel })
    }
}

/// `path` inside the space folder, checked; with its clean relative form.
fn object_path(root: &BlobPath, path: &str) -> Result<(BlobPath, String), Error> {
    let bad = |reason: &str| Error::new(ErrorKind::InvalidInput, format!("path: {reason}"));
    let rel = BlobPath::new(path.trim_matches('/')).map_err(|_| bad("not a valid path"))?;
    if rel.segments().any(|s| s == SYSTEM_DIR) {
        return Err(bad("Vellum's own files are not served here"));
    }
    let full = root
        .join(rel.as_str())
        .map_err(|_| bad("not a valid path"))?;
    if !full.starts_with(root) {
        return Err(bad("outside the space"));
    }
    Ok((full, rel.as_str().to_owned()))
}

/// `path` relative to `root`, or `None` for system files.
fn relative(root: &BlobPath, path: &BlobPath) -> Option<String> {
    let rel = path
        .as_str()
        .strip_prefix(root.as_str())?
        .trim_start_matches('/');
    if rel.is_empty() || rel.split('/').any(|s| s == SYSTEM_DIR) {
        return None;
    }
    Some(rel.to_owned())
}

fn modified(at: std::time::SystemTime) -> Timestamp {
    at.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .and_then(|ms| Timestamp::from_unix_ms(ms).ok())
        .unwrap_or_else(Timestamp::now)
}

fn clean_label(label: &str) -> Result<String, Error> {
    let label: String = label
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_owned();
    if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("label must be 1 to {MAX_LABEL_CHARS} characters"),
        ));
    }
    Ok(label)
}

fn busy() -> Error {
    Error::new(
        ErrorKind::Unavailable,
        "the space's key list is changing; try again",
    )
}

fn too_large() -> Error {
    Error::new(
        ErrorKind::InvalidInput,
        format!("envelope is larger than {MAX_ENVELOPE_BYTES} bytes"),
    )
}

fn store_error(error: StoreError) -> Error {
    match error {
        StoreError::NotFound(_) => Error::new(ErrorKind::NotFound, "no such object"),
        StoreError::AlreadyExists(_) => {
            Error::new(ErrorKind::Conflict, "the object already exists")
        }
        StoreError::PreconditionFailed { .. } => Error::new(
            ErrorKind::Conflict,
            "the object changed; fetch it and try again",
        ),
        StoreError::InvalidPath { .. } => Error::new(ErrorKind::InvalidInput, "not a valid path"),
        other => Error::new(ErrorKind::Unavailable, "the blob store failed").with_source(other),
    }
}
