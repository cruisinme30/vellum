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
///
/// Windows and Git add their own rules, because a path that is harmless
/// on one system escapes the folder on another (#272): no `:` (drive
/// prefixes and NTFS streams), no Windows device names (`CON`, `nul.md`,
/// `COM1`), no segment ending in `.` or a space (Windows drops them, so
/// two paths would name one file), and no `.git` segment in any spelling
/// the file system might fold to it (`.GIT`, `git~1`, `.g\u{200c}it`).
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
            s if s.contains(':') => return Some("colon (drive prefix or NTFS stream)"),
            s if s.ends_with(['.', ' ']) => return Some("segment ends in a dot or space"),
            s if is_windows_device(s) => return Some("Windows device name"),
            s if is_git_dir(s) => return Some("Git directory"),
            _ => {}
        }
    }
    None
}

/// Windows device names, which name a device whatever the extension.
const WINDOWS_DEVICES: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

fn is_windows_device(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or(segment).trim_end();
    let upper = stem.to_ascii_uppercase();
    if WINDOWS_DEVICES.contains(&upper.as_str()) || upper == "CONIN$" || upper == "CONOUT$" {
        return true;
    }
    // COM0-9 and LPT0-9, including the superscript digits Windows accepts.
    let mut chars = upper.chars();
    let prefix: String = chars.by_ref().take(3).collect();
    let digit = chars.next();
    (prefix == "COM" || prefix == "LPT")
        && matches!(digit, Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'))
        && chars.next().is_none()
}

/// Whether a file system might fold `segment` to `.git`: any case (macOS,
/// Windows), the NTFS short name `git~1`, and code points that HFS+
/// ignores when comparing names.
pub fn is_git_dir(segment: &str) -> bool {
    let folded: String = segment
        .chars()
        .filter(|c| !is_ignorable(*c))
        .collect::<String>()
        .to_ascii_lowercase();
    folded == ".git" || folded == "git~1"
}

/// Code points HFS+ skips when comparing names (as in CVE-2014-9390).
const fn is_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{200c}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{206a}'..='\u{206f}' | '\u{feff}'
    )
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
            // Windows: drive prefixes, streams, devices, trailing dots and spaces.
            "C:/Windows/win.ini",
            "C:",
            "notes/a.md:secret",
            "CON",
            "nul.md",
            "a/Aux.txt",
            "com1",
            "LPT9.log",
            "COM\u{b9}",
            "conin$",
            "a.md.",
            "a.md ",
            "dir./a.md",
            // Git: the repository directory in any spelling.
            ".git/config",
            ".GIT/hooks/post-checkout",
            "x/.git/config",
            "GIT~1/config",
            ".g\u{200c}it/config",
            "\u{feff}.git/config",
        ] {
            assert!(BlobPath::new(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn accepts_names_that_only_look_reserved() {
        for good in [
            "console.md",
            "Auxiliary/notes.md",
            "COM10.md",
            "nullable.md",
            ".gitignore",
            ".github/workflows/ci.yml",
            "my.git/a.md",
            "git/a.md",
            "Family \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}.md",
            "v1.2.md",
        ] {
            assert!(BlobPath::new(good).is_ok(), "{good:?} should be accepted");
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
