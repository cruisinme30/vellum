// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! AWS KMS as a key provider.
//!
//! DEKs are wrapped with KMS `Encrypt` and unwrapped with `Decrypt` through
//! the JSON API, signed with `SigV4`. The encryption context binds each
//! wrapped DEK to its KEK id. KEK ids look like
//! `aws-kms:eu-west-1:arn:aws:kms:eu-west-1:111122223333:key/…`.

use std::fmt;
use std::time::Instant;

use serde_json::json;
use vellum_crypto::{CryptoError, KekId, KeyProvider, SecretKey, WrappedDek};
use zeroize::Zeroizing;

use crate::aws_credentials::AwsCredentialProvider;
use crate::http::{self, b64, unb64};
use crate::sigv4;

const BACKEND: &str = "aws-kms";
const PREFIX: &str = "aws-kms:";
const CONTEXT_KEY: &str = "vellum";

/// Credentials for signing requests.
#[derive(Clone)]
pub struct AwsCredentials {
    pub(crate) access_key_id: String,
    pub(crate) secret_access_key: Zeroizing<String>,
    pub(crate) session_token: Option<Zeroizing<String>>,
}

impl fmt::Debug for AwsCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsCredentials")
            .field("access_key_id", &self.access_key_id)
            .finish_non_exhaustive()
    }
}

impl AwsCredentials {
    /// Long-term or temporary credentials. Pass the session token for
    /// temporary ones (STS, an assumed role).
    #[must_use]
    pub fn new(
        access_key_id: impl Into<String>,
        secret_access_key: impl Into<String>,
        session_token: Option<String>,
    ) -> Self {
        Self {
            access_key_id: access_key_id.into(),
            secret_access_key: Zeroizing::new(secret_access_key.into()),
            session_token: session_token.map(Zeroizing::new),
        }
    }

    /// Reads `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and the optional
    /// `AWS_SESSION_TOKEN`.
    ///
    /// # Errors
    /// [`CryptoError::Provider`] if either required variable is missing.
    pub fn from_env() -> Result<Self, CryptoError> {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let missing = |name: &str| CryptoError::Provider(format!("{BACKEND}: {name} is not set"));
        Ok(Self::new(
            var("AWS_ACCESS_KEY_ID").ok_or_else(|| missing("AWS_ACCESS_KEY_ID"))?,
            var("AWS_SECRET_ACCESS_KEY").ok_or_else(|| missing("AWS_SECRET_ACCESS_KEY"))?,
            var("AWS_SESSION_TOKEN"),
        ))
    }
}

/// Wraps DEKs with an AWS KMS symmetric key.
#[derive(Debug)]
pub struct AwsKms {
    client: reqwest::Client,
    credentials: AwsCredentialProvider,
    /// Overrides the regional endpoint (VPC endpoints, tests).
    endpoint: Option<String>,
    current: KekId,
}

impl AwsKms {
    /// A provider whose current KEK is `key_id` (a key id, key ARN or alias
    /// ARN) in `region`. Pass [`AwsCredentials`] for fixed keys, or an
    /// [`AwsCredentialProvider`] (usually [`AwsCredentialProvider::from_env`])
    /// for role credentials without static keys.
    ///
    /// # Errors
    /// [`CryptoError::InvalidKekId`] if the region or key id contain
    /// characters a KEK id cannot hold, or the HTTP client cannot be built.
    pub fn new(
        region: &str,
        key_id: &str,
        credentials: impl Into<AwsCredentialProvider>,
    ) -> Result<Self, CryptoError> {
        Ok(Self {
            client: http::client()?,
            credentials: credentials.into(),
            endpoint: None,
            current: kek_id(region, key_id)?,
        })
    }

    /// Sends every request to `endpoint` (for example
    /// `https://vpce-…kms.eu-west-1.vpce.amazonaws.com`) instead of
    /// `https://kms.<region>.amazonaws.com`.
    #[must_use]
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = Some(endpoint.into().trim_end_matches('/').to_owned());
        self
    }

    async fn call(
        &self,
        region: &str,
        target: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, CryptoError> {
        let endpoint = self
            .endpoint
            .clone()
            .unwrap_or_else(|| format!("https://kms.{region}.amazonaws.com"));
        let host = endpoint
            .split_once("://")
            .map_or(endpoint.as_str(), |(_, rest)| rest)
            .split('/')
            .next()
            .unwrap_or_default()
            .to_owned();
        let body = http::to_json(BACKEND, body)?;
        let credentials = self.credentials.credentials().await?;
        let amz_date = sigv4::amz_date_now();
        let target = format!("TrentService.{target}");
        let mut headers = vec![
            ("content-type", "application/x-amz-json-1.1"),
            ("host", host.as_str()),
            ("x-amz-date", amz_date.as_str()),
            ("x-amz-target", target.as_str()),
        ];
        if let Some(token) = &credentials.session_token {
            headers.push(("x-amz-security-token", token.as_str()));
        }
        let authorization = sigv4::authorization(
            &sigv4::Request {
                method: "POST",
                path: "/",
                query: "",
                headers: &headers,
                body: &body,
            },
            &sigv4::Scope {
                amz_date: &amz_date,
                region,
                service: "kms",
                access_key_id: &credentials.access_key_id,
                secret_access_key: &credentials.secret_access_key,
            },
        )?;
        let mut request = self.client.post(format!("{endpoint}/")).body(body);
        for (name, value) in headers {
            // reqwest derives Host from the URL.
            if name != "host" {
                request = request.header(name, value);
            }
        }
        request = request.header("authorization", authorization);
        http::send_json(BACKEND, request).await
    }
}

impl KeyProvider for AwsKms {
    fn current_kek(&self) -> KekId {
        self.current.clone()
    }

    #[tracing::instrument(name = "kms.wrap", skip_all, fields(backend = BACKEND, kek_id = %self.current))]
    async fn wrap(&self, dek: &SecretKey) -> Result<WrappedDek, CryptoError> {
        let started = Instant::now();
        let result = async {
            let (region, key) = parse(&self.current)?;
            let plaintext = Zeroizing::new(b64(dek.expose()));
            let body = json!({
                "KeyId": key,
                "Plaintext": plaintext.as_str(),
                "EncryptionContext": { CONTEXT_KEY: http::aad(&self.current) },
            });
            let response = self.call(region, "Encrypt", &body).await?;
            let blob = unb64(
                BACKEND,
                http::string_at(BACKEND, &response, "/CiphertextBlob")?,
            )?;
            Ok(WrappedDek {
                kek_id: self.current.clone(),
                blob,
            })
        }
        .await;
        http::record(BACKEND, "wrap", started, result)
    }

    #[tracing::instrument(name = "kms.unwrap", skip_all, fields(backend = BACKEND, kek_id = %wrapped.kek_id))]
    async fn unwrap(&self, wrapped: &WrappedDek) -> Result<SecretKey, CryptoError> {
        let started = Instant::now();
        let result = async {
            let (region, key) = parse(&wrapped.kek_id)?;
            let body = json!({
                "KeyId": key,
                "CiphertextBlob": b64(&wrapped.blob),
                "EncryptionContext": { CONTEXT_KEY: http::aad(&wrapped.kek_id) },
            });
            let response = self.call(region, "Decrypt", &body).await?;
            let plaintext = Zeroizing::new(unb64(
                BACKEND,
                http::string_at(BACKEND, &response, "/Plaintext")?,
            )?);
            http::dek(BACKEND, &plaintext)
        }
        .await;
        http::record(BACKEND, "unwrap", started, result)
    }
}

fn kek_id(region: &str, key_id: &str) -> Result<KekId, CryptoError> {
    if !http::safe_segment(region, &[]) || !http::safe_segment(key_id, &[':', '/']) {
        return Err(CryptoError::InvalidKekId);
    }
    KekId::new(format!("{PREFIX}{region}:{key_id}"))
}

/// Splits an `aws-kms:` KEK id into region and key id.
fn parse(id: &KekId) -> Result<(&str, &str), CryptoError> {
    id.as_str()
        .strip_prefix(PREFIX)
        .and_then(|rest| rest.split_once(':'))
        .filter(|(region, key)| {
            http::safe_segment(region, &[]) && http::safe_segment(key, &[':', '/'])
        })
        .ok_or_else(|| CryptoError::UnknownKek(id.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kek_ids_round_trip_through_parse() -> Result<(), CryptoError> {
        let arn = "arn:aws:kms:eu-west-1:111122223333:key/1234abcd-12ab-34cd-56ef-1234567890ab";
        let id = kek_id("eu-west-1", arn)?;
        assert_eq!(parse(&id)?, ("eu-west-1", arn));
        assert!(kek_id("eu west", "k").is_err());
        assert!(kek_id("eu-west-1", "key/../x").is_err());
        assert!(parse(&KekId::new("vault:transit/k")?).is_err());
        Ok(())
    }
}
