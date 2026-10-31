// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Book and section identifiers.
//!
//! Both are ULIDs: 26 characters of Crockford base32, as written in the
//! `id` front-matter field of every Vellum file. The id, never the path, is
//! a section's identity. New ids come from an [`IdSource`] so that the
//! caller decides how they are minted (the kernel's generator in the
//! server, a counter in tests).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{BookError, Result};

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn validate(value: &str) -> Result<String> {
    let upper = value.trim().to_ascii_uppercase();
    if upper.len() != 26 {
        return Err(BookError::InvalidId {
            value: value.to_owned(),
            reason: "a ULID has 26 characters",
        });
    }
    if !upper.bytes().all(|b| CROCKFORD.contains(&b)) {
        return Err(BookError::InvalidId {
            value: value.to_owned(),
            reason: "a ULID uses Crockford base32",
        });
    }
    // The first character carries the top 3 bits of a 48-bit timestamp.
    if upper.as_bytes().first().is_some_and(|b| *b > b'7') {
        return Err(BookError::InvalidId {
            value: value.to_owned(),
            reason: "the timestamp overflows 48 bits",
        });
    }
    Ok(upper)
}

macro_rules! ulid_newtype {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Parses and normalises (upper-cases) a ULID.
            ///
            /// # Errors
            ///
            /// Returns [`BookError::InvalidId`] when `value` is not a ULID.
            pub fn parse(value: &str) -> Result<Self> {
                validate(value).map(Self)
            }

            /// The canonical upper-case form.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = BookError;

            fn from_str(s: &str) -> Result<Self> {
                Self::parse(s)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(
                &self,
                serializer: S
            ) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(
                deserializer: D
            ) -> Result<Self, D::Error> {
                let raw = String::deserialize(deserializer)?;
                Self::parse(&raw).map_err(serde::de::Error::custom)
            }
        }
    };
}

ulid_newtype!(
    /// The identity of a book (`id` in `book.yaml`).
    BookId
);

ulid_newtype!(
    /// The identity of a section: the `id` in its file's front matter.
    NodeId
);

impl NodeId {
    /// The `vellum://doc/<id>` URI that links to this section.
    #[must_use]
    pub fn uri(&self) -> String {
        format!("{DOC_SCHEME}{}", self.0)
    }

    /// Extracts the id from a `vellum://doc/<id>` link destination,
    /// ignoring any `#fragment`.
    #[must_use]
    pub fn from_uri(dest: &str) -> Option<(Self, Option<&str>)> {
        let rest = dest.strip_prefix(DOC_SCHEME)?;
        let (id, fragment) = match rest.split_once('#') {
            Some((id, fragment)) => (id, Some(fragment)),
            None => (rest, None),
        };
        Self::parse(id).ok().map(|id| (id, fragment))
    }
}

/// The scheme and authority of document links stored in Vellum Markdown.
pub const DOC_SCHEME: &str = "vellum://doc/";

/// Mints fresh section ids, for new sections, copies and splits.
pub trait IdSource {
    /// Returns an id that has never been used.
    fn next_id(&mut self) -> NodeId;
}

impl<F: FnMut() -> NodeId> IdSource for F {
    fn next_id(&mut self) -> NodeId {
        self()
    }
}

/// An [`IdSource`] that counts upwards from a fixed timestamp; useful for
/// tests and deterministic fixtures, never for production data.
#[derive(Debug, Clone)]
pub struct SequentialIds {
    prefix: &'static str,
    next: u64,
}

impl SequentialIds {
    /// Ids start at `<prefix>` followed by a zero-padded counter.
    ///
    /// `prefix` must be 10 Crockford characters starting with 0-7.
    #[must_use]
    pub const fn new(prefix: &'static str) -> Self {
        Self { prefix, next: 1 }
    }
}

impl Default for SequentialIds {
    fn default() -> Self {
        Self::new("01JBK00000")
    }
}

impl IdSource for SequentialIds {
    fn next_id(&mut self) -> NodeId {
        let mut digits = String::with_capacity(16);
        let mut n = self.next;
        for _ in 0..16 {
            let index = usize::try_from(n % 32).unwrap_or_default();
            digits
                .insert(0, char::from(*CROCKFORD.get(index).unwrap_or(&b'0')));
            n /= 32;
        }
        self.next += 1;
        NodeId(format!("{}{digits}", self.prefix))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalises() -> Result<()> {
        let id = NodeId::parse("01jbk0000000000000000000a1")?;
        assert_eq!(id.as_str(), "01JBK0000000000000000000A1");
        Ok(())
    }

    #[test]
    fn rejects_bad_ids() {
        assert!(NodeId::parse("short").is_err());
        assert!(NodeId::parse("01JBK0000000000000000000I1").is_err());
        assert!(NodeId::parse("81JBK0000000000000000000A1").is_err());
    }

    #[test]
    fn uri_round_trip() -> Result<()> {
        let id = NodeId::parse("01JBK0000000000000000000A1")?;
        let uri = id.uri();
        assert_eq!(NodeId::from_uri(&uri), Some((id.clone(), None)));
        let with_fragment = format!("{uri}#setup");
        assert_eq!(NodeId::from_uri(&with_fragment), Some((id, Some("setup"))));
        assert_eq!(NodeId::from_uri("https://northstar.dev"), None);
        Ok(())
    }

    #[test]
    fn sequential_ids_are_valid_and_distinct() -> Result<()> {
        let mut ids = SequentialIds::default();
        let a = ids.next_id();
        let b = ids.next_id();
        assert_ne!(a, b);
        assert_eq!(NodeId::parse(a.as_str())?, a);
        Ok(())
    }
}
