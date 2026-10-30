// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Bearer secrets: random, prefixed, and stored only as a hash.
//!
//! Every credential the service hands out (session cookies, API tokens,
//! `OAuth` codes and tokens, break-glass codes) is 32 random bytes,
//! base64url-encoded behind a short prefix that says what it is, so a
//! leaked secret is recognisable by secret scanners. Only the SHA-256 of
//! the whole string is stored, so a copy of the store grants nothing.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};
use vellum_kernel::{Error, ErrorKind};

/// What a secret is for, which fixes its prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A browser session cookie.
    Session,
    /// A personal API token.
    ApiToken,
    /// An `OAuth` authorization code.
    AuthCode,
    /// An `OAuth` access token.
    AccessToken,
    /// An `OAuth` refresh token.
    RefreshToken,
    /// A one-time break-glass code.
    BreakGlass,
    /// A pending `OAuth` consent.
    Consent,
    /// A second-factor ticket or passkey ceremony (ADR 0069, D290).
    MfaTicket,
}

impl Kind {
    const ALL: [Self; 8] = [
        Self::Session,
        Self::ApiToken,
        Self::AuthCode,
        Self::AccessToken,
        Self::RefreshToken,
        Self::BreakGlass,
        Self::Consent,
        Self::MfaTicket,
    ];

    /// The prefix, including its underscore.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Session => "vss_",
            Self::ApiToken => "vlm_",
            Self::AuthCode => "vac_",
            Self::AccessToken => "vat_",
            Self::RefreshToken => "vrt_",
            Self::BreakGlass => "vbg_",
            Self::Consent => "vcn_",
            Self::MfaTicket => "vmt_",
        }
    }

    /// The kind a presented secret claims to be, from its prefix.
    #[must_use]
    pub fn of(secret: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| secret.starts_with(kind.prefix()))
    }
}

/// The stored form of a secret.
pub type SecretHash = [u8; 32];

/// Generates a new secret of `kind`.
///
/// # Errors
///
/// [`ErrorKind::Internal`] if the system random source fails.
pub fn generate(kind: Kind) -> Result<String, Error> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| Error::new(ErrorKind::Internal, format!("random source failed: {e}")))?;
    Ok(format!(
        "{}{}",
        kind.prefix(),
        URL_SAFE_NO_PAD.encode(bytes)
    ))
}

/// Hashes a presented secret for lookup.
#[must_use]
pub fn hash(secret: &str) -> SecretHash {
    Sha256::digest(secret.as_bytes()).into()
}

/// A short, stable, non-secret label for a hash, for logs, audit and IDs.
#[must_use]
pub fn label(hash: &SecretHash) -> String {
    URL_SAFE_NO_PAD.encode(hash.get(..9).unwrap_or_default())
}

/// Whether a presented string has the shape of a secret of `kind`: the
/// prefix and 43 base64url characters.
#[must_use]
pub fn well_formed(secret: &str, kind: Kind) -> bool {
    secret.strip_prefix(kind.prefix()).is_some_and(|rest| {
        rest.len() == 43
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_prefixed_random_and_hashable() {
        let a = generate(Kind::ApiToken).unwrap();
        let b = generate(Kind::ApiToken).unwrap();
        assert_ne!(a, b);
        assert!(a.starts_with("vlm_"));
        assert!(well_formed(&a, Kind::ApiToken));
        assert!(!well_formed(&a, Kind::Session));
        assert!(!well_formed("vlm_short", Kind::ApiToken));
        assert_eq!(Kind::of(&a), Some(Kind::ApiToken));
        assert_eq!(Kind::of("nope"), None);
        assert_eq!(hash(&a), hash(&a));
        assert_ne!(hash(&a), hash(&b));
        assert_eq!(label(&hash(&a)).len(), 12);
    }
}
