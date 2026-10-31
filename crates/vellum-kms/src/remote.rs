// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! One provider chosen from a KEK id at startup (ADR 0025, D260).

use vellum_crypto::{CryptoError, KekId, KeyProvider, SecretKey, WrappedDek};

use crate::{
    AwsCredentialProvider, AwsKms, GcpKms, MetadataServerToken, VaultTransit,
};

/// The remote key service a KEK id names.
#[derive(Debug)]
pub enum RemoteKeys {
    /// `aws-kms:<region>:<key id, key ARN or alias ARN>`.
    Aws(AwsKms),
    /// `gcp-kms:projects/<p>/locations/<l>/keyRings/<r>/cryptoKeys/<k>`.
    Gcp(GcpKms<MetadataServerToken>),
    /// `vault:<mount>/<key>`.
    Vault(VaultTransit),
}

impl RemoteKeys {
    /// Builds the provider `kek` names, with credentials from `var`:
    ///
    /// - AWS: the [`AwsCredentialProvider::from_vars`] chain, so EKS, ECS
    ///   and EC2 roles need no keys; `AWS_ENDPOINT_URL_KMS` overrides the
    ///   endpoint (VPC endpoints).
    /// - Google Cloud: the metadata server's service account (GKE workload
    ///   identity, GCE, Cloud Run).
    /// - Vault: `VAULT_ADDR`, `VAULT_TOKEN` and `VAULT_NAMESPACE`.
    ///
    /// # Errors
    /// [`CryptoError::InvalidKekId`] for an id no remote provider owns or a
    /// malformed one; [`CryptoError::Provider`] for missing settings.
    pub fn from_kek_id(
        kek: &str,
        var: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, CryptoError> {
        let var = |name: &str| var(name).filter(|v| !v.trim().is_empty());
        if let Some(rest) = kek.strip_prefix("aws-kms:") {
            let (region, key) =
                rest.split_once(':').ok_or(CryptoError::InvalidKekId)?;
            let credentials = AwsCredentialProvider::from_vars(var)?;
            tracing::info!(source = credentials.kind(), "AWS KMS credentials");
            let kms = AwsKms::new(region, key, credentials)?;
            return Ok(Self::Aws(match var("AWS_ENDPOINT_URL_KMS") {
                Some(endpoint) => kms.with_endpoint(endpoint),
                None => kms,
            }));
        }
        if let Some(name) = kek.strip_prefix("gcp-kms:") {
            return Ok(Self::Gcp(GcpKms::new(
                name,
                MetadataServerToken::new()?,
            )?));
        }
        if let Some(rest) = kek.strip_prefix("vault:") {
            let (mount, key) =
                rest.rsplit_once('/').ok_or(CryptoError::InvalidKekId)?;
            let missing = |name: &str| {
                CryptoError::Provider(format!(
                    "vault-transit: {name} is not set"
                ))
            };
            let addr =
                var("VAULT_ADDR").ok_or_else(|| missing("VAULT_ADDR"))?;
            let token =
                var("VAULT_TOKEN").ok_or_else(|| missing("VAULT_TOKEN"))?;
            let vault = VaultTransit::new(addr, token, mount, key)?;
            return Ok(Self::Vault(match var("VAULT_NAMESPACE") {
                Some(namespace) => vault.with_namespace(namespace),
                None => vault,
            }));
        }
        Err(CryptoError::InvalidKekId)
    }

    /// `aws-kms`, `gcp-kms` or `vault-transit`, for logs.
    #[must_use]
    pub const fn backend(&self) -> &'static str {
        match self {
            Self::Aws(_) => "aws-kms",
            Self::Gcp(_) => "gcp-kms",
            Self::Vault(_) => "vault-transit",
        }
    }
}

impl KeyProvider for RemoteKeys {
    fn current_kek(&self) -> KekId {
        match self {
            Self::Aws(p) => p.current_kek(),
            Self::Gcp(p) => p.current_kek(),
            Self::Vault(p) => p.current_kek(),
        }
    }

    async fn wrap(&self, dek: &SecretKey) -> Result<WrappedDek, CryptoError> {
        match self {
            Self::Aws(p) => p.wrap(dek).await,
            Self::Gcp(p) => p.wrap(dek).await,
            Self::Vault(p) => p.wrap(dek).await,
        }
    }

    async fn unwrap(
        &self,
        wrapped: &WrappedDek,
    ) -> Result<SecretKey, CryptoError> {
        match self {
            Self::Aws(p) => p.unwrap(wrapped).await,
            Self::Gcp(p) => p.unwrap(wrapped).await,
            Self::Vault(p) => p.unwrap(wrapped).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(
        pairs: &'a [(&'a str, &'a str)],
    ) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn kek_ids_choose_the_backend() -> Result<(), CryptoError> {
        let aws = RemoteKeys::from_kek_id(
            "aws-kms:eu-west-1:alias/vellum",
            vars(&[("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "/v2/c")]),
        )?;
        assert_eq!(aws.backend(), "aws-kms");
        assert_eq!(
            aws.current_kek().as_str(),
            "aws-kms:eu-west-1:alias/vellum"
        );
        let gcp = RemoteKeys::from_kek_id(
            "gcp-kms:projects/p/locations/l/keyRings/r/cryptoKeys/k",
            vars(&[]),
        )?;
        assert_eq!(gcp.backend(), "gcp-kms");
        let vault = RemoteKeys::from_kek_id(
            "vault:transit/vellum",
            vars(&[("VAULT_ADDR", "https://vault"), ("VAULT_TOKEN", "t")]),
        )?;
        assert_eq!(vault.current_kek().as_str(), "vault:transit/vellum");
        Ok(())
    }

    #[test]
    fn unknown_or_incomplete_ids_are_refused() {
        assert!(RemoteKeys::from_kek_id("argon2id$m=1", vars(&[])).is_err());
        assert!(RemoteKeys::from_kek_id("aws-kms:no-key", vars(&[])).is_err());
        assert!(RemoteKeys::from_kek_id("vault:transit/k", vars(&[])).is_err());
    }
}
