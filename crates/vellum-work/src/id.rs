// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Identifiers used by Work objects.
//!
//! Object IDs are the kernel's typed IDs, written as ULID text (D37). The
//! engine never generates IDs: callers pass them in, so the engine stays
//! deterministic. People are referenced by handle ([`UserRef`]).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::WorkError;

pub use vellum_kernel::{
    InitiativeId, IssueId, MilestoneId, ProjectId, ProjectUpdateId as UpdateId, TeamId, ViewId,
};

/// A person, written as their handle (`dev`, `priya`).
///
/// Handles are lowercase letters, digits, `.`, `_` and `-`, and start with
/// a letter or digit. The identity service maps a handle to a user ID.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct UserRef(String);

impl UserRef {
    /// Parses a handle, accepting an optional leading `@`.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] if the handle is empty, too long or
    /// uses other characters.
    pub fn parse(text: &str) -> Result<Self, WorkError> {
        let handle = text.strip_prefix('@').unwrap_or(text);
        let invalid = |reason| WorkError::Invalid {
            what: "user handle",
            value: text.to_owned(),
            reason,
        };
        if handle.is_empty() || handle.len() > 64 {
            return Err(invalid("must be 1 to 64 characters"));
        }
        let ok = handle.bytes().enumerate().all(|(i, b)| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || (i > 0 && matches!(b, b'.' | b'_' | b'-'))
        });
        if !ok {
            return Err(invalid(
                "must be lowercase letters, digits, '.', '_' or '-'",
            ));
        }
        Ok(Self(handle.to_owned()))
    }

    /// The handle without the `@`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UserRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for UserRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.0)
    }
}

impl<'de> Deserialize<'de> for UserRef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_strip_the_at_sign() {
        assert_eq!(UserRef::parse("@priya").unwrap().as_str(), "priya");
        assert!(UserRef::parse("Priya").is_err());
        assert!(UserRef::parse("-x").is_err());
        assert!(UserRef::parse("").is_err());
    }
}
