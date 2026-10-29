// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Webhook signing secrets.
//!
//! A secret is 32 bytes from the operating system's CSPRNG, shown to the
//! admin once as `whsec_` followed by base64url. Vellum stores it sealed by
//! the envelope encryption layer; it never appears in logs because
//! [`fmt::Debug`] is redacted.

use std::fmt;

use crate::encoding::{b64url, unb64url};

/// The prefix that marks a webhook secret in configuration and UIs.
pub const PREFIX: &str = "whsec_";

/// Length of a secret in bytes.
pub const LEN: usize = 32;

/// A webhook signing secret.
#[derive(Clone, PartialEq, Eq)]
pub struct WebhookSecret([u8; LEN]);

/// Why a secret could not be made or read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// The operating system's random source failed.
    #[error("the system random source failed")]
    Random,
    /// The text is not `whsec_` plus 32 bytes of base64url.
    #[error("not a webhook secret")]
    Malformed,
}

impl WebhookSecret {
    /// Generates a fresh secret.
    ///
    /// # Errors
    ///
    /// [`SecretError::Random`] if the system random source fails.
    pub fn generate() -> Result<Self, SecretError> {
        let mut bytes = [0_u8; LEN];
        getrandom::fill(&mut bytes).map_err(|_| SecretError::Random)?;
        Ok(Self(bytes))
    }

    /// Wraps existing key bytes, for example after unsealing.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; LEN]) -> Self {
        Self(bytes)
    }

    /// Parses the `whsec_…` form.
    ///
    /// # Errors
    ///
    /// [`SecretError::Malformed`] for anything else.
    pub fn parse(text: &str) -> Result<Self, SecretError> {
        let body = text.strip_prefix(PREFIX).ok_or(SecretError::Malformed)?;
        let bytes = unb64url(body).ok_or(SecretError::Malformed)?;
        let array: [u8; LEN] = bytes.try_into().map_err(|_| SecretError::Malformed)?;
        Ok(Self(array))
    }

    /// The key bytes, for HMAC.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; LEN] {
        &self.0
    }

    /// The `whsec_…` form, shown once to the admin who creates the hook.
    #[must_use]
    pub fn reveal(&self) -> String {
        format!("{PREFIX}{}", b64url(&self.0))
    }
}

impl fmt::Debug for WebhookSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WebhookSecret(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_parse_round_trip() -> Result<(), SecretError> {
        let secret = WebhookSecret::generate()?;
        let text = secret.reveal();
        assert!(text.starts_with(PREFIX));
        assert_eq!(WebhookSecret::parse(&text)?, secret);
        assert_ne!(WebhookSecret::generate()?, secret);
        Ok(())
    }

    #[test]
    fn rejects_malformed() {
        assert_eq!(WebhookSecret::parse("abc"), Err(SecretError::Malformed));
        assert_eq!(
            WebhookSecret::parse("whsec_abc"),
            Err(SecretError::Malformed)
        );
    }

    #[test]
    fn debug_is_redacted() {
        let secret = WebhookSecret::from_bytes([7; LEN]);
        assert_eq!(format!("{secret:?}"), "WebhookSecret(<redacted>)");
    }
}
