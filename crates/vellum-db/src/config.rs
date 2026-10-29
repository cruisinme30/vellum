// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! How to reach the index.

use std::fmt;
use std::fmt::Write as _;
use std::path::PathBuf;

/// A 256-bit SQLCipher key. Its `Debug` output never shows the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct IndexKey([u8; 32]);

impl IndexKey {
    /// Wraps raw key bytes, already unwrapped from the KEK by the caller.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The `PRAGMA key` value: a raw key in SQLCipher's `x'…'` blob syntax,
    /// which skips SQLCipher's own passphrase derivation.
    pub(crate) fn pragma_value(&self) -> String {
        let mut out = String::with_capacity(67);
        out.push_str("\"x'");
        for byte in self.0 {
            let _ = write!(out, "{byte:02X}");
        }
        out.push_str("'\"");
        out
    }
}

impl fmt::Debug for IndexKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IndexKey(<redacted>)")
    }
}

/// Which backend to open, and how.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum IndexConfig {
    /// Zero-config single-binary profile: a SQLite file, normally `.vellum/index.sqlite`.
    Sqlite {
        /// Path of the database file; created if missing.
        path: PathBuf,
        /// SQLCipher key. `None` only when the operator chose plaintext mode.
        key: Option<IndexKey>,
    },
    /// HA profiles: a Postgres connection URL.
    Postgres {
        /// `postgres://` URL.
        url: String,
        /// Upper bound on pooled connections.
        max_connections: u32,
    },
}

impl IndexConfig {
    /// SQLite at `path`, encrypted with `key`.
    #[must_use]
    pub const fn sqlite(path: PathBuf, key: Option<IndexKey>) -> Self {
        Self::Sqlite { path, key }
    }

    /// Postgres at `url` with the default pool size of 16.
    #[must_use]
    pub fn postgres(url: impl Into<String>) -> Self {
        Self::Postgres {
            url: url.into(),
            max_connections: 16,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::IndexKey;

    #[test]
    fn key_is_hex_blob_and_redacted() {
        let key = IndexKey::new([0xAB; 32]);
        let pragma = key.pragma_value();
        assert_eq!(pragma.len(), 3 + 64 + 2);
        assert!(pragma.starts_with("\"x'AB"));
        assert_eq!(format!("{key:?}"), "IndexKey(<redacted>)");
    }
}
