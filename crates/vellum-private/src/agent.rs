// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The agent: opens private spaces on this device and searches them.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use async_trait::async_trait;
use futures::TryStreamExt as _;
use tokio::sync::RwLock;
use tracing::Instrument as _;
use vellum_crypto::CryptoError;
use vellum_crypto::recipients::{is_recipient_envelope, recipients_of};
use vellum_kernel::{DocId, ObjectId, SpaceId, Timestamp, UserId};
use vellum_search::{
    IndexKey, Location, Principal, ReadAccess, SearchError, SearchHit, SearchQuery, SearchService,
    Stars, TantivyBackend, TantivyConfig, Update,
};
use vellum_store::layout::{FOLDER_META, Kind};
use vellum_store::{BlobPath, BlobStore, StoreError};

use crate::extract::{PrivateDoc, read_document};
use crate::identity::DeviceIdentity;

/// The person searches run as. The agent only holds what this device could
/// open, so every document in it is readable by its user.
const LOCAL_USER: UserId = UserId::from_u128(1);

/// A private space on this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateSpace {
    /// The space's ID, from its `.vellum-folder.yaml`.
    pub id: SpaceId,
    /// A short name, used in MCP calls and citations.
    pub slug: String,
    /// Its folder in the store.
    pub root: BlobPath,
}

impl PrivateSpace {
    /// Reads the space at `root`: its ID comes from the folder's
    /// `.vellum-folder.yaml`, in plain text or sealed to this device.
    ///
    /// # Errors
    ///
    /// The folder has no readable manifest or the manifest has no valid
    /// `id`.
    pub async fn open(
        store: &dyn BlobStore,
        identity: &DeviceIdentity,
        slug: &str,
        root: BlobPath,
    ) -> Result<Self, AgentError> {
        let manifest = root.join(FOLDER_META)?;
        let bytes = store.get(&manifest).await?.bytes;
        let bytes = if is_recipient_envelope(&bytes) {
            identity.open(&bytes)?
        } else {
            bytes.to_vec()
        };
        let text = String::from_utf8(bytes)
            .map_err(|_| AgentError::NotASpace(format!("{root}: the manifest is not text")))?;
        let meta: Manifest = serde_saphyr::from_str(&text)
            .map_err(|e| AgentError::NotASpace(format!("{root}: {e}")))?;
        let id: ObjectId = meta
            .id
            .parse()
            .map_err(|_| AgentError::NotASpace(format!("{root}: the manifest id is not valid")))?;
        Ok(Self {
            id: id.into(),
            slug: slug.to_owned(),
            root,
        })
    }
}

#[derive(serde::Deserialize)]
struct Manifest {
    id: String,
}

/// What a refresh found.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Report {
    /// Pages opened and indexed.
    pub opened: usize,
    /// Pages sealed to other devices only: this device is not enrolled for
    /// them (yet), or was removed.
    pub not_for_this_device: usize,
    /// Pages that should have opened but did not: tampered, damaged, not
    /// text, or without an ID.
    pub unreadable: usize,
    /// Files in a private space that are not sealed at all. A private space
    /// should hold none; they are left out rather than trusted.
    pub not_sealed: usize,
}

/// Why the agent failed.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// The store failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// The search index failed.
    #[error(transparent)]
    Search(#[from] SearchError),
    /// A key or envelope problem.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    /// A folder named as a private space is not one.
    #[error("not a private space: {0}")]
    NotASpace(String),
    /// Two spaces were given the same short name.
    #[error("two private spaces are named `{0}`")]
    DuplicateSlug(String),
}

/// Opens private spaces on this device and searches them.
pub struct PrivateAgent {
    identity: DeviceIdentity,
    store: Arc<dyn BlobStore>,
    spaces: Vec<PrivateSpace>,
    docs: RwLock<BTreeMap<DocId, Arc<PrivateDoc>>>,
    search: SearchService,
}

impl std::fmt::Debug for PrivateAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrivateAgent")
            .field("identity", &self.identity)
            .field("spaces", &self.spaces)
            .finish_non_exhaustive()
    }
}

impl PrivateAgent {
    /// An agent over `spaces` in `store`, with an empty index. Call
    /// [`Self::refresh`] to open the pages.
    ///
    /// The index lives in memory, sealed with a key made for this process
    /// alone, so nothing the agent decrypts reaches the disk.
    ///
    /// # Errors
    ///
    /// Two spaces share a slug, the random source fails, or the index
    /// cannot be created.
    pub fn new(
        identity: DeviceIdentity,
        store: Arc<dyn BlobStore>,
        spaces: Vec<PrivateSpace>,
    ) -> Result<Self, AgentError> {
        let mut seen = HashSet::new();
        if let Some(dup) = spaces.iter().find(|s| !seen.insert(s.slug.clone())) {
            return Err(AgentError::DuplicateSlug(dup.slug.clone()));
        }
        let key = vellum_crypto::recipients::RecipientSecret::generate()?;
        let (backend, _) = TantivyBackend::open(&TantivyConfig {
            location: Location::Memory,
            key: Some(IndexKey::from_bytes(*key.expose())),
            model: None,
        })?;
        let readable = Everything(spaces.iter().map(|s| s.id).collect());
        let search = SearchService::new(Arc::new(backend), Arc::new(readable), Arc::new(NoStars));
        Ok(Self {
            identity,
            store,
            spaces,
            docs: RwLock::default(),
            search,
        })
    }

    /// This device's identity.
    #[must_use]
    pub const fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    /// The spaces the agent serves.
    #[must_use]
    pub fn spaces(&self) -> &[PrivateSpace] {
        &self.spaces
    }

    /// The space with this slug.
    #[must_use]
    pub fn space(&self, slug: &str) -> Option<&PrivateSpace> {
        self.spaces.iter().find(|s| s.slug == slug)
    }

    /// Opens every page sealed to this device and rebuilds the index.
    ///
    /// # Errors
    ///
    /// The store or the index failed. A page that does not open is counted
    /// in the report, not an error.
    pub async fn refresh(&self) -> Result<Report, AgentError> {
        let span = tracing::info_span!(
            "private.refresh",
            spaces = self.spaces.len(),
            opened = tracing::field::Empty,
            not_for_this_device = tracing::field::Empty,
            unreadable = tracing::field::Empty,
        );
        async {
            let mut report = Report::default();
            let mut docs = BTreeMap::new();
            for space in &self.spaces {
                self.open_space(space, &mut report, &mut docs).await?;
            }
            self.search.clear().await?;
            self.search
                .write(
                    docs.values()
                        .map(|d| Update::Upsert(d.to_index()))
                        .collect(),
                )
                .await?;
            *self.docs.write().await = docs.into_iter().map(|(id, d)| (id, Arc::new(d))).collect();
            let span = tracing::Span::current();
            let _ = span.record("opened", report.opened);
            let _ = span.record("not_for_this_device", report.not_for_this_device);
            let _ = span.record("unreadable", report.unreadable);
            Ok(report)
        }
        .instrument(span)
        .await
    }

    async fn open_space(
        &self,
        space: &PrivateSpace,
        report: &mut Report,
        docs: &mut BTreeMap<DocId, PrivateDoc>,
    ) -> Result<(), AgentError> {
        let entries: Vec<_> = self.store.list(Some(&space.root)).try_collect().await?;
        let mine = self.identity.fingerprint();
        for entry in entries {
            if Kind::of(&entry.path) != Kind::Document {
                continue;
            }
            // One page that cannot be fetched (deleted since the listing,
            // or refused by a server as not end-to-end encrypted) must not
            // stop the whole pass.
            let sealed = match self.store.get(&entry.path).await {
                Ok(blob) => blob.bytes,
                Err(StoreError::NotFound(_)) => continue,
                Err(error) => {
                    report.unreadable += 1;
                    tracing::warn!(path = %entry.path, %error, "skipped a private page that could not be fetched");
                    continue;
                }
            };
            if !is_recipient_envelope(&sealed) {
                report.not_sealed += 1;
                tracing::warn!(path = %entry.path, "skipped a file that is not sealed in a private space");
                continue;
            }
            let plain = match self.identity.open(&sealed) {
                Ok(plain) => plain,
                Err(_) if !recipients_of(&sealed).is_ok_and(|r| r.contains(&mine)) => {
                    report.not_for_this_device += 1;
                    continue;
                }
                Err(error) => {
                    report.unreadable += 1;
                    tracing::warn!(path = %entry.path, %error, "a private page did not open");
                    continue;
                }
            };
            let modified = entry
                .modified
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|d| i64::try_from(d.as_millis()).ok())
                .and_then(|ms| Timestamp::from_unix_ms(ms).ok())
                .unwrap_or_else(Timestamp::now);
            let Ok(text) = String::from_utf8(plain) else {
                report.unreadable += 1;
                continue;
            };
            match read_document(space.id, entry.path.as_str(), text, modified) {
                Ok(doc) => {
                    report.opened += 1;
                    let _ = docs.insert(doc.id, doc);
                }
                Err(error) => {
                    report.unreadable += 1;
                    tracing::warn!(%error, "a private page could not be read");
                }
            }
        }
        Ok(())
    }

    /// Ranked block hits for `text`, in one space or all of them.
    ///
    /// # Errors
    ///
    /// The index failed.
    #[tracing::instrument(name = "private.search", skip(self, text), fields(chars = text.len()))]
    pub async fn search(
        &self,
        text: &str,
        space: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchHit>, AgentError> {
        let mut query = SearchQuery::new(text, limit.clamp(1, vellum_search::model::MAX_LIMIT));
        if let Some(slug) = space {
            match self.space(slug) {
                Some(space) => query.filters.spaces = vec![space.id],
                None => return Ok(Vec::new()),
            }
        }
        let page = self
            .search
            .search(&Principal::User(LOCAL_USER), &query)
            .await?;
        Ok(page.hits)
    }

    /// The page with this ID.
    pub async fn document(&self, id: DocId) -> Option<Arc<PrivateDoc>> {
        self.docs.read().await.get(&id).cloned()
    }

    /// The page at this path in the store.
    pub async fn document_by_path(&self, path: &str) -> Option<Arc<PrivateDoc>> {
        self.docs
            .read()
            .await
            .values()
            .find(|d| d.path == path)
            .cloned()
    }

    /// Every page of a space, by path.
    pub async fn documents(&self, slug: &str) -> Vec<Arc<PrivateDoc>> {
        let Some(space) = self.space(slug) else {
            return Vec::new();
        };
        let mut docs: Vec<_> = self
            .docs
            .read()
            .await
            .values()
            .filter(|d| d.space == space.id)
            .cloned()
            .collect();
        docs.sort_by(|a, b| a.path.cmp(&b.path));
        docs
    }
}

/// Every space the agent serves, and every page in them: the agent only
/// holds pages this device could open.
struct Everything(Vec<SpaceId>);

#[async_trait]
impl ReadAccess for Everything {
    async fn readable_spaces(&self, _principal: &Principal) -> vellum_search::Result<Vec<SpaceId>> {
        Ok(self.0.clone())
    }

    async fn filter_readable(
        &self,
        _principal: &Principal,
        docs: &[DocId],
    ) -> vellum_search::Result<HashSet<DocId>> {
        Ok(docs.iter().copied().collect())
    }
}

struct NoStars;

#[async_trait]
impl Stars for NoStars {
    async fn starred(&self, _user: UserId) -> vellum_search::Result<HashSet<DocId>> {
        Ok(HashSet::new())
    }
}
