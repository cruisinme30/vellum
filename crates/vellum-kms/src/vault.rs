// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `HashiCorp` Vault's transit secrets engine as a key provider.
//!
//! DEKs are wrapped with `POST /v1/<mount>/encrypt/<key>` and unwrapped with
//! `.../decrypt/<key>`. Use an AEAD key type (`aes256-gcm96` or
//! `chacha20-poly1305`): the KEK id travels as `associated_data`. KEK ids
//! look like `vault:transit/vellum`. Rotating the transit key inside Vault
//! needs nothing from Vellum (Vault keeps old versions for decryption); a
//! different key gets a new KEK id and envelopes move with
//! [`vellum_crypto::rewrap`].

use std::fmt;
use std::time::Instant;

use serde_json::json;
use vellum_crypto::{CryptoError, KekId, KeyProvider, SecretKey, WrappedDek};
use zeroize::Zeroizing;

use crate::http::{self, b64, unb64};

const BACKEND: &str = "vault";
const PREFIX: &str = "vault:";

/// Wraps DEKs with a Vault transit key.
pub struct VaultTransit {
    client: reqwest::Client,
    addr: String,
    token: Zeroizing<String>,
    namespace: Option<String>,
    current: KekId,
}

impl fmt::Debug for VaultTransit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VaultTransit")
            .field("addr", &self.addr)
            .field("namespace", &self.namespace)
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}

impl VaultTransit {
    /// A provider talking to Vault at `addr` (for example
    /// `https://vault.internal:8200`) with `token`, whose current KEK is
    /// transit key `key` under `mount` (usually `transit`).
    ///
    /// # Errors
    /// [`CryptoError::InvalidKekId`] if `mount` or `key` are not plain path
    /// segments, or the HTTP client cannot be built.
    pub fn new(
        addr: impl Into<String>,
        token: impl Into<String>,
        mount: &str,
        key: &str,
    ) -> Result<Self, CryptoError> {
        Ok(Self {
            client: http::client()?,
            addr: addr.into().trim_end_matches('/').to_owned(),
            token: Zeroizing::new(token.into()),
            namespace: None,
            current: kek_id(mount, key)?,
        })
    }

    /// Reads `VAULT_ADDR`, `VAULT_TOKEN` and the optional `VAULT_NAMESPACE`,
    /// the variables the Vault CLI uses.
    ///
    /// # Errors
    /// [`CryptoError::Provider`] if `VAULT_ADDR` or `VAULT_TOKEN` is missing,
    /// or any error of [`VaultTransit::new`].
    pub fn from_env(mount: &str, key: &str) -> Result<Self, CryptoError> {
        let var =
            |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let missing = |name: &str| {
            CryptoError::Provider(format!("{BACKEND}: {name} is not set"))
        };
        let addr = var("VAULT_ADDR").ok_or_else(|| missing("VAULT_ADDR"))?;
        let token = var("VAULT_TOKEN").ok_or_else(|| missing("VAULT_TOKEN"))?;
        let provider = Self::new(addr, token, mount, key)?;
        Ok(match var("VAULT_NAMESPACE") {
            Some(namespace) => provider.with_namespace(namespace),
            None => provider,
        })
    }

    /// Sends `X-Vault-Namespace` (Vault Enterprise and HCP Vault).
    #[must_use]
    pub fn with_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.namespace = Some(namespace.into());
        self
    }

    async fn call(
        &self,
        op: &str,
        id: &KekId,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, CryptoError> {
        let (mount, key) = parse(id)?;
        let mut request = self
            .client
            .post(format!("{}/v1/{mount}/{op}/{key}", self.addr))
            .header("x-vault-token", self.token.as_str())
            .header("content-type", "application/json")
            .body(http::to_json(BACKEND, body)?);
        if let Some(namespace) = &self.namespace {
            request = request.header("x-vault-namespace", namespace);
        }
        http::send_json(BACKEND, request).await
    }
}

impl KeyProvider for VaultTransit {
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
                "associated_data": b64(http::aad(&self.current).as_bytes()),
            });
            let response = self.call("encrypt", &self.current, &body).await?;
            let ciphertext =
                http::string_at(BACKEND, &response, "/data/ciphertext")?;
            Ok(WrappedDek {
                kek_id: self.current.clone(),
                blob: ciphertext.as_bytes().to_vec(),
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
            let ciphertext =
                std::str::from_utf8(&wrapped.blob).map_err(|_| {
                    CryptoError::Malformed("vault ciphertext is not text")
                })?;
            let body = json!({
                "ciphertext": ciphertext,
                "associated_data": b64(http::aad(&wrapped.kek_id).as_bytes()),
            });
            let response = self.call("decrypt", &wrapped.kek_id, &body).await?;
            let plaintext = Zeroizing::new(unb64(
                BACKEND,
                http::string_at(BACKEND, &response, "/data/plaintext")?,
            )?);
            http::dek(BACKEND, &plaintext)
        }
        .await;
        http::record(BACKEND, "unwrap", started, result)
    }
}

fn kek_id(mount: &str, key: &str) -> Result<KekId, CryptoError> {
    if !http::safe_segment(mount, &['/']) || !http::safe_segment(key, &[]) {
        return Err(CryptoError::InvalidKekId);
    }
    KekId::new(format!("{PREFIX}{mount}/{key}"))
}

/// Splits a `vault:` KEK id into mount and key; the key is the last segment.
fn parse(id: &KekId) -> Result<(&str, &str), CryptoError> {
    id.as_str()
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.rsplit_once('/'))
        .filter(|(mount, key)| {
            http::safe_segment(mount, &['/']) && http::safe_segment(key, &[])
        })
        .ok_or_else(|| CryptoError::UnknownKek(id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kek_ids_round_trip_through_parse() -> Result<(), CryptoError> {
        let id = kek_id("team/transit", "vellum")?;
        assert_eq!(id.as_str(), "vault:team/transit/vellum");
        assert_eq!(parse(&id)?, ("team/transit", "vellum"));
        assert!(kek_id("transit", "a/b").is_err());
        assert!(kek_id("../sys", "k").is_err());
        assert!(parse(&KekId::new("vault:nokey")?).is_err());
        Ok(())
    }
}
