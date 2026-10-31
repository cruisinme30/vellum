// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Share-link tokens (R-SE-3).
//!
//! A token is 22 base62 characters drawn from the operating system's CSPRNG
//! (about 131 bits). Only its SHA-256 hash is stored; the plaintext is shown
//! once, in the link. Expiry and revocation live on the share record
//! ([`crate::ShareId`]), not in the token.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

const BASE62: &[u8; 62] =
    b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
/// Bytes at or above this value are rejected so `byte % 62` stays uniform.
const REJECT_FROM: u8 = 248;

/// Why a share token could not be generated or parsed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ShareTokenError {
    /// The operating system's random source failed.
    #[error("system random source failed: {0}")]
    Random(String),
    /// The text is not 22 base62 characters.
    #[error("share token must be {} base62 characters", ShareToken::LEN)]
    Malformed,
    /// The text is not 64 hexadecimal digits.
    #[error("share token hash must be 64 hex digits")]
    MalformedHash,
}

/// A plaintext share token. Its `Debug` output is redacted and its memory
/// is wiped on drop.
#[derive(Clone, PartialEq, Eq)]
pub struct ShareToken(Zeroizing<String>);

impl ShareToken {
    /// Number of characters in a token.
    pub const LEN: usize = 22;

    /// Generates a fresh token from the operating system's CSPRNG.
    ///
    /// # Errors
    ///
    /// Returns [`ShareTokenError::Random`] if the random source fails.
    pub fn generate() -> Result<Self, ShareTokenError> {
        let mut token = Zeroizing::new(String::with_capacity(Self::LEN));
        let mut buf = Zeroizing::new([0_u8; 32]);
        while token.len() < Self::LEN {
            getrandom::fill(buf.as_mut_slice())
                .map_err(|e| ShareTokenError::Random(e.to_string()))?;
            for &byte in buf.iter().filter(|&&b| b < REJECT_FROM) {
                if token.len() == Self::LEN {
                    break;
                }
                let c =
                    BASE62.get(usize::from(byte % 62)).copied().unwrap_or(b'0');
                token.push(char::from(c));
            }
        }
        Ok(Self(token))
    }

    /// Returns the plaintext, for building the link shown to the user once.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Returns the SHA-256 hash to store and look up.
    #[must_use]
    pub fn hash(&self) -> ShareTokenHash {
        ShareTokenHash(Sha256::digest(self.0.as_bytes()).into())
    }
}

impl FromStr for ShareToken {
    type Err = ShareTokenError;

    /// Accepts a token presented in a link, checking only its shape.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() == Self::LEN
            && text.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            Ok(Self(Zeroizing::new(text.to_owned())))
        } else {
            Err(ShareTokenError::Malformed)
        }
    }
}

impl fmt::Debug for ShareToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ShareToken(<redacted>)")
    }
}

/// The SHA-256 hash of a [`ShareToken`], as stored. Equality is constant-time.
#[derive(Clone, Copy, Eq)]
pub struct ShareTokenHash([u8; 32]);

impl ShareTokenHash {
    /// Wraps stored hash bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the hash bytes for storage.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns whether `token` hashes to this value, in constant time.
    #[must_use]
    pub fn verify(&self, token: &ShareToken) -> bool {
        *self == token.hash()
    }
}

impl PartialEq for ShareTokenHash {
    fn eq(&self, other: &Self) -> bool {
        self.0.ct_eq(&other.0).into()
    }
}

impl std::hash::Hash for ShareTokenHash {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

impl fmt::Display for ShareTokenHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for ShareTokenHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ShareTokenHash({self})")
    }
}

impl FromStr for ShareTokenHash {
    type Err = ShareTokenError;

    /// Parses 64 hexadecimal digits (either case).
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() != 64 || !text.is_ascii() {
            return Err(ShareTokenError::MalformedHash);
        }
        let mut bytes = [0_u8; 32];
        for (byte, pair) in
            bytes.iter_mut().zip(text.as_bytes().chunks_exact(2))
        {
            let pair = std::str::from_utf8(pair)
                .map_err(|_| ShareTokenError::MalformedHash)?;
            *byte = u8::from_str_radix(pair, 16)
                .map_err(|_| ShareTokenError::MalformedHash)?;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for ShareTokenHash {
    fn serialize<S: Serializer>(
        &self,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ShareTokenHash {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_have_the_right_shape() -> Result<(), ShareTokenError> {
        let token = ShareToken::generate()?;
        assert_eq!(token.expose().len(), ShareToken::LEN);
        assert!(token.expose().bytes().all(|b| b.is_ascii_alphanumeric()));
        assert_ne!(token, ShareToken::generate()?);
        Ok(())
    }

    #[test]
    fn hash_matches_a_known_vector() -> Result<(), ShareTokenError> {
        let token: ShareToken = "0123456789abcdefABCDEF".parse()?;
        // printf %s 0123456789abcdefABCDEF | sha256sum
        let expected: ShareTokenHash =
            "1f55b9dbb34db31ee615fea98af1b62ec6f169bb38355164364099813349e079"
                .parse()?;
        assert_eq!(token.hash(), expected);
        assert_eq!(
            expected.to_string(),
            "1f55b9dbb34db31ee615fea98af1b62ec6f169bb38355164364099813349e079"
        );
        assert!(expected.verify(&token));
        Ok(())
    }

    #[test]
    fn debug_never_shows_the_token() -> Result<(), ShareTokenError> {
        let token: ShareToken = "0123456789abcdefABCDEF".parse()?;
        assert_eq!(format!("{token:?}"), "ShareToken(<redacted>)");
        Ok(())
    }

    #[test]
    fn rejects_malformed_tokens() {
        for bad in [
            "",
            "short",
            "0123456789abcdefABCDE!",
            "0123456789abcdefABCDEFG",
            "0123456789abcdefABCDé",
        ] {
            assert_eq!(
                bad.parse::<ShareToken>(),
                Err(ShareTokenError::Malformed),
                "{bad:?}"
            );
        }
    }
}
