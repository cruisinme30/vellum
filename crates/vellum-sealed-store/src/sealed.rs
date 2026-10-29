// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The encrypting [`BlobStore`] decorator.

use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use opentelemetry::KeyValue;
use opentelemetry::metrics::Counter;
use vellum_crypto::{CryptoError, KeyProvider};
use vellum_store::{
    Blob, BlobEntry, BlobMeta, BlobPath, BlobStore, Precondition, StoreError, StoreResult,
};

use crate::policy::{Mode, ModePolicy};

/// Counter of plaintext objects read where the policy says encrypted,
/// labelled `backend`. Non-zero means a space still needs [`Sealed::reseal`].
pub const PLAINTEXT_READS: &str = "vellum_sealed_plaintext_reads_total";

/// Wraps a store so objects in encrypted spaces are sealed on write and
/// opened on read.
///
/// - Etags are the inner store's (of the stored bytes), so preconditions
///   pass straight through.
/// - `get`, `head`, `put` and `rename` report plaintext sizes; `list`
///   reports stored sizes, because listings never read content.
/// - Envelopes are not bound to their path (ADR 0002, D35), so renames and
///   moves between spaces keep working.
#[derive(Debug)]
pub struct Sealed<S, K, P> {
    inner: S,
    keys: Arc<K>,
    policy: P,
    plaintext_reads: Counter<u64>,
}

impl<S: BlobStore, K: KeyProvider + std::fmt::Debug, P: ModePolicy> Sealed<S, K, P> {
    /// Wraps `inner`, sealing with `keys` wherever `policy` says encrypted.
    pub fn new(inner: S, keys: Arc<K>, policy: P) -> Self {
        let plaintext_reads = opentelemetry::global::meter("vellum")
            .u64_counter(PLAINTEXT_READS)
            .with_description("Plaintext objects read in encrypted spaces")
            .build();
        Self {
            inner,
            keys,
            policy,
            plaintext_reads,
        }
    }

    /// The wrapped store.
    pub const fn inner(&self) -> &S {
        &self.inner
    }

    /// Rewrites one object into the form the policy wants now: seals
    /// plaintext in an encrypted space, re-wraps envelopes whose KEK is not
    /// current (rotation), and opens envelopes in a plaintext space. Uses
    /// `IfMatch`, so a concurrent writer wins and the caller retries.
    /// Returns whether anything was written.
    ///
    /// # Errors
    /// Store errors, including `PreconditionFailed` on a race, and
    /// `Backend` for encryption failures.
    #[tracing::instrument(name = "sealed.reseal", skip(self), fields(path = %path))]
    pub async fn reseal(&self, path: &BlobPath) -> StoreResult<bool> {
        let blob = self.inner.get(path).await?;
        let sealed = vellum_crypto::is_envelope(&blob.bytes);
        let next = match (self.policy.mode_for(path), sealed) {
            (Mode::Encrypted, true) => vellum_crypto::rewrap(self.keys.as_ref(), &blob.bytes)
                .await
                .map_err(crypto_error)?,
            (Mode::Encrypted, false) => Some(
                vellum_crypto::seal(self.keys.as_ref(), &blob.bytes)
                    .await
                    .map_err(crypto_error)?,
            ),
            (Mode::Plaintext, true) => Some(
                vellum_crypto::open(self.keys.as_ref(), &blob.bytes)
                    .await
                    .map_err(crypto_error)?,
            ),
            (Mode::Plaintext, false) => None,
        };
        let Some(next) = next else {
            return Ok(false);
        };
        let condition = Precondition::IfMatch(blob.meta.etag);
        let _ = self.inner.put(path, Bytes::from(next), condition).await?;
        Ok(true)
    }

    async fn decode(&self, path: &BlobPath, stored: Bytes) -> StoreResult<Bytes> {
        if vellum_crypto::is_envelope(&stored) {
            let plain = vellum_crypto::open(self.keys.as_ref(), &stored)
                .await
                .map_err(crypto_error)?;
            return Ok(Bytes::from(plain));
        }
        if self.policy.mode_for(path) == Mode::Encrypted {
            self.plaintext_reads
                .add(1, &[KeyValue::new("backend", self.inner.backend())]);
            tracing::warn!(path = %path, "plaintext object in an encrypted space; reseal it");
        }
        Ok(stored)
    }
}

#[async_trait]
impl<S, K, P> BlobStore for Sealed<S, K, P>
where
    S: BlobStore,
    K: KeyProvider + std::fmt::Debug,
    P: ModePolicy,
{
    fn backend(&self) -> &'static str {
        self.inner.backend()
    }

    async fn get(&self, path: &BlobPath) -> StoreResult<Blob> {
        let blob = self.inner.get(path).await?;
        let bytes = self.decode(path, blob.bytes).await?;
        let meta = BlobMeta {
            size: len(&bytes),
            ..blob.meta
        };
        Ok(Blob { bytes, meta })
    }

    async fn head(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        // The plaintext size lives in the header, so this reads the object;
        // callers that only need the version should keep using etags.
        let blob = self.inner.get(path).await?;
        let size = plain_size(&blob.bytes)?;
        Ok(BlobMeta { size, ..blob.meta })
    }

    async fn put(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        condition: Precondition,
    ) -> StoreResult<BlobMeta> {
        let size = len(&bytes);
        let stored = match self.policy.mode_for(path) {
            Mode::Encrypted => Bytes::from(
                vellum_crypto::seal(self.keys.as_ref(), &bytes)
                    .await
                    .map_err(crypto_error)?,
            ),
            Mode::Plaintext => bytes,
        };
        let meta = self.inner.put(path, stored, condition).await?;
        Ok(BlobMeta { size, ..meta })
    }

    async fn delete(&self, path: &BlobPath, condition: Precondition) -> StoreResult<()> {
        self.inner.delete(path, condition).await
    }

    async fn rename(&self, from: &BlobPath, to: &BlobPath) -> StoreResult<BlobMeta> {
        let _ = self.inner.rename(from, to).await?;
        self.head(to).await
    }

    fn list(&self, dir: Option<&BlobPath>) -> BoxStream<'static, StoreResult<BlobEntry>> {
        self.inner.list(dir)
    }
}

fn plain_size(stored: &[u8]) -> StoreResult<u64> {
    if vellum_crypto::is_envelope(stored) {
        let n = vellum_crypto::plaintext_len(stored).map_err(crypto_error)?;
        return Ok(u64::try_from(n).unwrap_or(u64::MAX));
    }
    Ok(len(stored))
}

fn len(bytes: &[u8]) -> u64 {
    u64::try_from(bytes.len()).unwrap_or(u64::MAX)
}

#[allow(clippy::needless_pass_by_value)] // used as a `map_err` callback
fn crypto_error(error: CryptoError) -> StoreError {
    StoreError::Backend(format!("encryption at rest: {error}"))
}
