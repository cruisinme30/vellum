// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Block IDs: stable anchors for blocks inside one document.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::base32;
use crate::id::IdParseError;

/// Number of Crockford characters in a block ID body.
const BLOCK_DIGITS: u32 = 8;
const BLOCK_PREFIX: &str = "blk-";

/// A block inside a document, written `^blk-xxxxxxxx` in Vellum Markdown.
///
/// The body is 8 Crockford base32 characters (40 random bits), lowercase in
/// canonical form. Block IDs only need to be unique within one document.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(u64);

impl BlockId {
    /// The largest value a block ID can hold (40 bits).
    pub const MAX: u64 = (1 << 40) - 1;

    /// Generates a random block ID from the operating system's CSPRNG.
    #[must_use]
    pub fn generate() -> Self {
        // A v4 UUID carries 122 random bits; the low 40 are all random.
        // Truncation to 40 bits is the intent.
        #[allow(clippy::cast_possible_truncation)]
        let bits = (Uuid::new_v4().as_u128() as u64) & Self::MAX;
        Self(bits)
    }

    /// Builds a block ID from a value, or `None` if it exceeds 40 bits.
    #[must_use]
    pub const fn from_u64(value: u64) -> Option<Self> {
        if value > Self::MAX {
            None
        } else {
            Some(Self(value))
        }
    }

    /// Returns the 40-bit value.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(BLOCK_PREFIX)?;
        f.write_str(
            &base32::encode(u128::from(self.0), BLOCK_DIGITS)
                .to_ascii_lowercase(),
        )
    }
}

impl fmt::Debug for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlockId({self})")
    }
}

impl FromStr for BlockId {
    type Err = IdParseError;

    /// Parses `blk-xxxxxxxx`; the body is case-insensitive.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        const KIND: &str = "BlockId";
        let body =
            text.strip_prefix(BLOCK_PREFIX)
                .ok_or(IdParseError::Prefix {
                    kind: KIND,
                    prefix: BLOCK_PREFIX,
                })?;
        let actual = body.chars().count();
        if actual != BLOCK_DIGITS as usize {
            return Err(IdParseError::Length {
                kind: KIND,
                expected: BLOCK_DIGITS as usize,
                actual,
            });
        }
        let value = base32::decode(body).map_err(
            |base32::DecodeError::Character { found, position }| {
                IdParseError::Character {
                    kind: KIND,
                    found,
                    position: position + BLOCK_PREFIX.len(),
                }
            },
        )?;
        // Eight digits hold exactly 40 bits, so the value always fits.
        u64::try_from(value)
            .ok()
            .and_then(Self::from_u64)
            .ok_or(IdParseError::Overflow { kind: KIND })
    }
}

impl schemars::JsonSchema for BlockId {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "BlockId".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^blk-[0-9a-hjkmnp-tv-zA-HJKMNP-TV-Z]{8}$",
            "description": "A block ID: blk- and 8 Crockford base32 characters.",
        })
    }
}

impl Serialize for BlockId {
    fn serialize<S: Serializer>(
        &self,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for BlockId {
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
    fn block_id_text_form() {
        let id = BlockId::from_u64(0).unwrap_or_else(|| unreachable!());
        assert_eq!(id.to_string(), "blk-00000000");
        assert_eq!(
            "BLK-0000000Z".parse::<BlockId>().err(),
            Some(IdParseError::Prefix {
                kind: "BlockId",
                prefix: "blk-"
            })
        );
        assert_eq!(
            "blk-0000000Z".parse::<BlockId>().map(BlockId::as_u64),
            Ok(31)
        );
        assert_eq!(BlockId::from_u64(BlockId::MAX + 1), None);
        assert!(matches!(
            "blk-0000000u".parse::<BlockId>(),
            Err(IdParseError::Character {
                found: 'u',
                position: 11,
                ..
            })
        ));
    }
}
