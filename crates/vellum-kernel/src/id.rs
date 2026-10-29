// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Typed identifiers.
//!
//! Every object ID is 128 bits generated as a `UUIDv7` (48-bit millisecond
//! timestamp, then random bits) and written as a 26-character ULID in
//! Crockford base32. The same value is a valid `uuid` column in Postgres and
//! a valid ULID in front matter, and both forms sort by creation time.
//! Parsing accepts any ULID, so files written by other tools still load.
//!
//! Each kind of object has its own newtype, so a [`DocId`] can never be
//! passed where a [`SpaceId`] is expected.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use crate::base32;

/// JSON Schema pattern for ULID text (case-insensitive Crockford base32).
const ULID_PATTERN: &str = "^[0-7][0-9A-HJKMNP-TV-Za-hjkmnp-tv-z]{25}$";

/// Length of an object ID in its ULID text form.
pub const ULID_LEN: usize = 26;

/// Why a string is not a valid ID.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum IdParseError {
    /// The text has the wrong number of characters.
    #[error("{kind} must be {expected} characters, got {actual}")]
    Length {
        /// The ID type being parsed.
        kind: &'static str,
        /// The required length.
        expected: usize,
        /// The length that was given.
        actual: usize,
    },
    /// The text contains a character outside the Crockford alphabet.
    #[error("{kind} has invalid character {found:?} at position {position}")]
    Character {
        /// The ID type being parsed.
        kind: &'static str,
        /// The offending character.
        found: char,
        /// Its zero-based character position.
        position: usize,
    },
    /// The first character is above `7`, so the value exceeds 128 bits.
    #[error("{kind} is out of range: the first character must be 0-7")]
    Overflow {
        /// The ID type being parsed.
        kind: &'static str,
    },
    /// The text does not start with the required prefix.
    #[error("{kind} must start with {prefix:?}")]
    Prefix {
        /// The ID type being parsed.
        kind: &'static str,
        /// The required prefix.
        prefix: &'static str,
    },
}

/// Parses ULID text into its 128-bit value.
fn parse_ulid(kind: &'static str, text: &str) -> Result<u128, IdParseError> {
    let actual = text.chars().count();
    if actual != ULID_LEN {
        return Err(IdParseError::Length {
            kind,
            expected: ULID_LEN,
            actual,
        });
    }
    let value =
        base32::decode(text).map_err(|base32::DecodeError::Character { found, position }| {
            IdParseError::Character {
                kind,
                found,
                position,
            }
        })?;
    // 26 digits hold 130 bits; a first digit above 7 sets the top two.
    if text.starts_with(|c| base32::decode_digit(c).is_some_and(|d| d > 7)) {
        return Err(IdParseError::Overflow { kind });
    }
    Ok(value)
}

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, typed) => {
        define_id!($(#[$meta])* $name);

        impl From<$name> for ObjectId {
            fn from(id: $name) -> Self {
                Self::from_u128(id.as_u128())
            }
        }

        impl From<ObjectId> for $name {
            /// Types an untyped ID once its kind is known (from its folder, say).
            fn from(id: ObjectId) -> Self {
                Self::from_u128(id.as_u128())
            }
        }
    };
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u128);

        impl $name {
            /// Generates a new, time-ordered ID (`UUIDv7` layout).
            #[must_use]
            pub fn generate() -> Self {
                Self(Uuid::now_v7().as_u128())
            }

            /// Wraps a raw 128-bit value, such as one read from storage.
            #[must_use]
            pub const fn from_u128(value: u128) -> Self {
                Self(value)
            }

            /// Returns the raw 128-bit value.
            #[must_use]
            pub const fn as_u128(self) -> u128 {
                self.0
            }

            /// Wraps a UUID, such as one read from a Postgres `uuid` column.
            #[must_use]
            pub const fn from_uuid(uuid: Uuid) -> Self {
                Self(uuid.as_u128())
            }

            /// Returns the ID as a UUID for storage.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                Uuid::from_u128(self.0)
            }

            /// Milliseconds since the Unix epoch encoded in the top 48 bits.
            #[must_use]
            pub const fn timestamp_ms(self) -> u64 {
                // Shifted down to 48 bits, so the narrowing cannot truncate.
                #[allow(clippy::cast_possible_truncation)]
                let ms = (self.0 >> 80) as u64;
                ms
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&base32::encode(self.0, 26))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), self)
            }
        }

        impl FromStr for $name {
            type Err = IdParseError;

            fn from_str(text: &str) -> Result<Self, Self::Err> {
                parse_ulid(stringify!($name), text).map(Self)
            }
        }

        impl schemars::JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({
                    "type": "string",
                    "format": "ulid",
                    "pattern": ULID_PATTERN,
                    "description": concat!("A ", stringify!($name), " as a 26-character ULID."),
                })
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Self {
                id.as_uuid()
            }
        }

        impl From<Uuid> for $name {
            fn from(uuid: Uuid) -> Self {
                Self::from_uuid(uuid)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

/// Defines a list of ID types; `typed` ones convert to and from [`ObjectId`].
macro_rules! define_ids {
    ($($(#[$meta:meta])* $name:ident $($typed:ident)?;)*) => {
        $(define_id!($(#[$meta])* $name $(, $typed)?);)*
    };
}

define_ids! {
    /// Any object's front-matter `id`, before its kind is known.
    ///
    /// The watcher and indexer read this from files; convert it to a typed
    /// ID with `From` once the object's kind is known.
    ObjectId;
    /// A space: the shard key for every table and path.
    SpaceId typed;
    /// A document (page, book leaf, collection row, issue body); the `id` in front matter.
    DocId typed;
    /// A book (`name.book/`).
    BookId typed;
    /// A collection (`name.collection/`).
    CollectionId typed;
    /// A drive: a shared drive or a user's My Drive.
    DriveId typed;
    /// A folder inside a drive (`.vellum-folder.yaml`).
    FolderId typed;
    /// A user account.
    UserId typed;
    /// A team that owns issues and workflows.
    TeamId typed;
    /// An issue. Its human key (`ENG-142`) is separate and can change.
    IssueId typed;
    /// A comment on a document or issue.
    CommentId typed;
    /// A stored version of a document.
    VersionId typed;
    /// A My Work task projected from a Markdown checklist block.
    TaskId typed;
    /// A share link. The secret token is separate; this is its record.
    ShareId typed;
    /// A stored blob (attachment, image, rendered output).
    BlobId typed;
    /// A domain event.
    EventId;
    /// A request, propagated in headers, logs and traces.
    RequestId;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_prd_example() {
        let id: DocId = "01JB3K9Q7Z5N8M2X4V6T0R1Y3W"
            .parse()
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(id.to_string(), "01JB3K9Q7Z5N8M2X4V6T0R1Y3W");
        assert_eq!(format!("{id:?}"), "DocId(01JB3K9Q7Z5N8M2X4V6T0R1Y3W)");
    }

    #[test]
    fn lowercase_ulid_normalises_to_uppercase() {
        let id: SpaceId = "01jb3k9q7z5n8m2x4v6t0r1y3w"
            .parse()
            .unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(id.to_string(), "01JB3K9Q7Z5N8M2X4V6T0R1Y3W");
    }

    #[test]
    fn generated_ids_are_uuid_v7() {
        let id = DocId::generate();
        assert_eq!(id.as_uuid().get_version_num(), 7);
        assert!(id.timestamp_ms() > 1_700_000_000_000);
    }

    #[test]
    fn rejects_bad_ulids() {
        let err = |s: &str| s.parse::<DocId>().err();
        assert!(matches!(
            err("01JB"),
            Some(IdParseError::Length { actual: 4, .. })
        ));
        assert!(matches!(
            err("01JB3K9Q7Z5N8M2X4V6T0R1Y3U"),
            Some(IdParseError::Character {
                found: 'U',
                position: 25,
                ..
            })
        ));
        assert!(matches!(
            err("81JB3K9Q7Z5N8M2X4V6T0R1Y3W"),
            Some(IdParseError::Overflow { .. })
        ));
        assert_eq!(
            "7ZZZZZZZZZZZZZZZZZZZZZZZZZ"
                .parse::<DocId>()
                .map(DocId::as_u128),
            Ok(u128::MAX)
        );
    }
}
