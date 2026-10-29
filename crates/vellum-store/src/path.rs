// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Validated, backend-neutral blob paths.

use std::fmt;

use crate::error::StoreError;

/// Prefix reserved for in-flight temporary files. Paths using it are
/// rejected, and adapters hide such files from listings and watchers.
pub const TEMP_PREFIX: &str = ".vellum-tmp-";

/// A relative, `/`-separated path to a blob inside a store.
///
/// Every adapter maps the same `BlobPath` to the same object, so the rules
/// are the intersection of what local folders, S3, Google Drive and Git
/// accept: no empty, `.` or `..` segments, no leading or trailing `/`, no
/// backslash, NUL or control characters, and at most 1024 bytes.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobPath(String);

/// Longest accepted path in bytes (the S3 key limit).
pub const MAX_PATH_BYTES: usize = 1024;

impl BlobPath {
    /// Validates `raw` and wraps it.
    ///
    /// # Errors
    /// Returns [`StoreError::InvalidPath`] when `raw` breaks a rule above.
    pub fn new(raw: impl Into<String>) -> Result<Self, StoreError> {
        let raw = raw.into();
        match violation(&raw) {
            Some(reason) => Err(StoreError::InvalidPath { path: raw, reason }),
            None => Ok(Self(raw)),
        }
    }

    /// The path as a `/`-separated string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The path's segments, outermost first.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    /// The final segment (the file name).
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// The containing directory, or `None` at the store root.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        self.0.rsplit_once('/').map(|(dir, _)| Self(dir.to_owned()))
    }

    /// Appends one or more segments.
    ///
    /// # Errors
    /// Returns [`StoreError::InvalidPath`] when the result is invalid.
    pub fn join(&self, rest: &str) -> Result<Self, StoreError> {
        Self::new(format!("{}/{rest}", self.0))
    }

    /// True when `self` is `dir` or lies beneath it.
    #[must_use]
    pub fn starts_with(&self, dir: &Self) -> bool {
        self.0 == dir.0
            || (self.0.starts_with(&dir.0) && self.0.as_bytes().get(dir.0.len()) == Some(&b'/'))
    }
}

fn violation(raw: &str) -> Option<&'static str> {
    if raw.is_empty() {
        return Some("empty path");
    }
    if raw.len() > MAX_PATH_BYTES {
        return Some("longer than 1024 bytes");
    }
    if raw.chars().any(|c| c == '\\' || c.is_control()) {
        return Some("backslash or control character");
    }
    for segment in raw.split('/') {
        match segment {
            "" => return Some("empty segment or leading/trailing slash"),
            "." | ".." => return Some("relative segment"),
            s if s.starts_with(TEMP_PREFIX) => return Some("reserved temporary-file prefix"),
            _ => {}
        }
    }
    None
}

impl fmt::Display for BlobPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for BlobPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlobPath({:?})", self.0)
    }
}

impl TryFrom<&str> for BlobPath {
    type Error = StoreError;

    fn try_from(raw: &str) -> Result<Self, Self::Error> {
        Self::new(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn accepts_nested_unicode_paths() {
        let p = BlobPath::new("drives/shared/Ops/Runbook – v2.md");
        assert!(p.is_ok_and(|p| p.file_name() == "Runbook – v2.md"));
    }

    #[test]
    fn rejects_escapes_and_oddities() {
        for bad in [
            "",
            "/a",
            "a/",
            "a//b",
            "../a",
            "a/./b",
            "a\\b",
            "a\0b",
            ".vellum-tmp-x",
        ] {
            assert!(BlobPath::new(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn parent_and_starts_with() -> Result<(), StoreError> {
        let p = BlobPath::new("a/b/c.md")?;
        assert_eq!(p.parent(), Some(BlobPath::new("a/b")?));
        assert!(p.starts_with(&BlobPath::new("a")?));
        assert!(!p.starts_with(&BlobPath::new("a/b/c")?));
        assert!(!BlobPath::new("ab/c")?.starts_with(&BlobPath::new("a")?));
        Ok(())
    }

    proptest! {
        #[test]
        fn valid_paths_never_escape(raw in "[a-z./]{1,24}") {
            if let Ok(p) = BlobPath::new(raw) {
                prop_assert!(p.segments().all(|s| !s.is_empty() && s != ".." && s != "."));
            }
        }
    }
}
