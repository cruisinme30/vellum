// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Google Cloud KMS as a key provider.
//!
//! DEKs are wrapped with `cryptoKeys.encrypt` and unwrapped with
//! `cryptoKeys.decrypt`; the KEK id travels as additional authenticated
//! data. KEK ids look like
//! `gcp-kms:projects/p/locations/l/keyRings/r/cryptoKeys/k`. Cloud KMS keeps
//! old key versions for decryption, so rotating inside KMS needs nothing
//! from Vellum.
//!
//! Access tokens come from a [`TokenSource`]: [`MetadataServerToken`] on
//! GCE, GKE and Cloud Run, or [`StaticToken`] for a token minted elsewhere.

use std::fmt;
use std::future::Future;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::json;
use vellum_crypto::{CryptoError, KekId, KeyProvider, SecretKey, WrappedDek};
use zeroize::Zeroizing;

use crate::http::{self, b64, unb64};

const BACKEND: &str = "gcp-kms";
const PREFIX: &str = "gcp-kms:";
const ENDPOINT: &str = "https://cloudkms.googleapis.com";
const METADATA_TOKEN_URL: &str = "http://metadata.google.internal/computeMetada\
    ta/v1/instance/service-accounts/default/token";
/// Refresh a cached token this long before it expires.
const REFRESH_MARGIN: Duration = Duration::from_mins(1);

/// Supplies `OAuth` 2.0 access tokens with the `cloudkms` scope.
pub trait TokenSource: Send + Sync {
    /// A token valid for at least the next request.
    fn token(
        &self,
    ) -> impl Future<Output = Result<Zeroizing<String>, CryptoError>> + Send;
}

/// A fixed access token, for example from `gcloud auth print-access-token`
/// or a sidecar that refreshes the provider.
pub struct StaticToken(Zeroizing<String>);

impl StaticToken {
    /// Wraps `token`.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self(Zeroizing::new(token.into()))
    }
}

impl fmt::Debug for StaticToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StaticToken(..)")
    }
}

impl TokenSource for StaticToken {
    fn token(
        &self,
    ) -> impl Future<Output = Result<Zeroizing<String>, CryptoError>> + Send
    {
        std::future::ready(Ok(self.0.clone()))
    }
}

/// Tokens for the attached service account from the GCE metadata server,
/// cached until shortly before they expire.
pub struct MetadataServerToken {
    client: reqwest::Client,
    url: String,
    cached: Mutex<Option<(Zeroizing<String>, Instant)>>,
}

impl fmt::Debug for MetadataServerToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetadataServerToken")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl MetadataServerToken {
    /// Uses the standard metadata server address.
    ///
    /// # Errors
    /// [`CryptoError::Provider`] if the HTTP client cannot be built.
    pub fn new() -> Result<Self, CryptoError> {
        Self::with_url(METADATA_TOKEN_URL)
    }

    /// Uses another token URL (tests, emulators).
    ///
    /// # Errors
    /// [`CryptoError::Provider`] if the HTTP client cannot be built.
    pub fn with_url(url: impl Into<String>) -> Result<Self, CryptoError> {
        Ok(Self {
            client: http::client()?,
            url: url.into(),
            cached: Mutex::new(None),
        })
    }

    fn cached(&self) -> Option<Zeroizing<String>> {
        let guard = self.cached.lock().ok()?;
        guard
            .as_ref()
            .filter(|(_, expires)| Instant::now() + REFRESH_MARGIN < *expires)
            .map(|(token, _)| token.clone())
    }

    async fn fetch(&self) -> Result<Zeroizing<String>, CryptoError> {
        let request = self
            .client
            .get(&self.url)
            .header("metadata-flavor", "Google");
        let body = http::send_json(BACKEND, request).await?;
        let token = Zeroizing::new(
            http::string_at(BACKEND, &body, "/access_token")?.to_owned(),
        );
        let lifetime = body
            .pointer("/expires_in")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if let Ok(mut guard) = self.cached.lock() {
            *guard = Some((
                token.clone(),
                Instant::now() + Duration::from_secs(lifetime),
            ));
        }
        Ok(token)
    }
}

impl TokenSource for MetadataServerToken {
    async fn token(&self) -> Result<Zeroizing<String>, CryptoError> {
        match self.cached() {
            Some(token) => Ok(token),
            None => self.fetch().await,
        }
    }
}

/// Wraps DEKs with a Cloud KMS symmetric key.
pub struct GcpKms<T> {
    client: reqwest::Client,
    endpoint: String,
    tokens: T,
    current: KekId,
}

impl<T> fmt::Debug for GcpKms<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GcpKms")
            .field("endpoint", &self.endpoint)
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}

impl<T: TokenSource> GcpKms<T> {
    /// A provider whose current KEK is the crypto key `key_name`
    /// (`projects/<p>/locations/<l>/keyRings/<r>/cryptoKeys/<k>`).
    ///
    /// # Errors
    /// [`CryptoError::InvalidKekId`] if `key_name` does not have that shape,
    /// or the HTTP client cannot be built.
    pub fn new(key_name: &str, tokens: T) -> Result<Self, CryptoError> {
        Ok(Self {
            client: http::client()?,
            endpoint: ENDPOINT.to_owned(),
            tokens,
            current: kek_id(key_name)?,
        })
    }

    /// Sends requests to `endpoint` instead of Cloud KMS (private service
    /// connect, tests).
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        endpoint
            .into()
            .trim_end_matches('/')
            .clone_into(&mut self.endpoint);
        self
    }

    async fn call(
        &self,
        op: &str,
        id: &KekId,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, CryptoError> {
        let name = parse(id)?;
        let token = self.tokens.token().await?;
        let request = self
            .client
            .post(format!("{}/v1/{name}:{op}", self.endpoint))
            .bearer_auth(token.as_str())
            .header("content-type", "application/json")
            .body(http::to_json(BACKEND, body)?);
        http::send_json(BACKEND, request).await
    }
}

impl<T: TokenSource> KeyProvider for GcpKms<T> {
    fn current_kek(&self) -> KekId {
        self.current.clone()
    }

    #[tracing::instrument(
        name = "kms.wrap",
        skip_all,
        fields(backend = BACKEND, kek_id = %self.current)
    )]
    async fn wrap(&self, dek: &SecretKey) -> Result<WrappedDek, CryptoError> {
        let started = Instant::now();
        let result = async {
            let plaintext = Zeroizing::new(b64(dek.expose()));
            let body = json!({
                "plaintext": plaintext.as_str(),
                "additionalAuthenticatedData": b64(
                    http::aad(&self.current).as_bytes()
                ),
            });
            let response = self.call("encrypt", &self.current, &body).await?;
            let blob = unb64(
                BACKEND,
                http::string_at(BACKEND, &response, "/ciphertext")?,
            )?;
            Ok(WrappedDek {
                kek_id: self.current.clone(),
                blob,
            })
        }
        .await;
        http::record(BACKEND, "wrap", started, result)
    }

    #[tracing::instrument(
        name = "kms.unwrap",
        skip_all,
        fields(backend = BACKEND, kek_id = %wrapped.kek_id)
    )]
    async fn unwrap(
        &self,
        wrapped: &WrappedDek,
    ) -> Result<SecretKey, CryptoError> {
        let started = Instant::now();
        let result = async {
            let body = json!({
                "ciphertext": b64(&wrapped.blob),
                "additionalAuthenticatedData": b64(
                    http::aad(&wrapped.kek_id).as_bytes()
                ),
            });
            let response = self.call("decrypt", &wrapped.kek_id, &body).await?;
            let plaintext = Zeroizing::new(unb64(
                BACKEND,
                http::string_at(BACKEND, &response, "/plaintext")?,
            )?);
            http::dek(BACKEND, &plaintext)
        }
        .await;
        http::record(BACKEND, "unwrap", started, result)
    }
}

fn kek_id(key_name: &str) -> Result<KekId, CryptoError> {
    if !valid_key_name(key_name) {
        return Err(CryptoError::InvalidKekId);
    }
    KekId::new(format!("{PREFIX}{key_name}"))
}

fn parse(id: &KekId) -> Result<&str, CryptoError> {
    id.as_str()
        .strip_prefix(PREFIX)
        .filter(|name| valid_key_name(name))
        .ok_or_else(|| CryptoError::UnknownKek(id.clone()))
}

/// `projects/*/locations/*/keyRings/*/cryptoKeys/*` with plain segments.
fn valid_key_name(name: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    let [
        "projects",
        project,
        "locations",
        location,
        "keyRings",
        ring,
        "cryptoKeys",
        key,
    ] = parts.as_slice()
    else {
        return false;
    };
    [project, location, ring, key]
        .iter()
        .all(|part| http::safe_segment(part, &[]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_must_be_crypto_keys() -> Result<(), CryptoError> {
        let name = "projects/northstar/locations/europe-west1/keyRings/vellum/c\
            ryptoKeys/kek";
        let id = kek_id(name)?;
        assert_eq!(parse(&id)?, name);
        assert!(kek_id("projects/p/locations/l/keyRings/r").is_err());
        assert!(
            kek_id(
                "projects/p/locations/l/keyRings/r/cryptoKeys/k/cryptoKeyVer\
                sions/1"
            )
            .is_err()
        );
        assert!(
            kek_id("projects/../locations/l/keyRings/r/cryptoKeys/k").is_err()
        );
        Ok(())
    }
}
