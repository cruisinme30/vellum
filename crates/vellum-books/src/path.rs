// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Chapter paths inside a `name.book/` folder.
//!
//! A [`ChapterPath`] is always relative to the book folder, uses `/`, ends
//! in `.md` and can never climb out of the folder: no `..`, no absolute
//! paths, no drive letters, no hidden segments. This is what keeps a
//! hostile `SUMMARY.md` from reading or writing outside its book.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{BookError, Result};

/// The file that holds a section with children, or the book's introduction.
pub const README: &str = "README.md";

/// A validated chapter path relative to the book folder.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChapterPath(String);

impl ChapterPath {
    /// Validates `value` as a chapter path.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::InvalidPath`] when the path is empty, absolute,
    /// climbs with `..`, has hidden or empty segments, contains control
    /// characters, backslashes or colons, or does not end in `.md`.
    pub fn parse(value: &str) -> Result<Self> {
        let fail = |reason| BookError::InvalidPath {
            path: value.to_owned(),
            reason,
        };
        let path = value.strip_prefix("./").unwrap_or(value);
        if path.is_empty() || path.len() > 1024 {
            return Err(fail("must be 1 to 1024 bytes"));
        }
        if path.starts_with('/') {
            return Err(fail("must be relative"));
        }
        if path
            .chars()
            .any(|c| c.is_control() || matches!(c, '\\' | ':' | '<' | '>' | '#' | '?' | '%' | '"' | '|' | '*'))
        {
            return Err(fail("control characters and \\ : < > # ? % \" | * are not allowed"));
        }
        for segment in path.split('/') {
            if segment.is_empty() {
                return Err(fail("empty segment"));
            }
            if segment.starts_with('.') {
                return Err(fail("segments may not start with '.'"));
            }
        }
        let has_md = std::path::Path::new(path)
            .extension()
            .is_some_and(|ext| ext == "md");
        if !has_md || path.ends_with("/.md") {
            return Err(fail("must name a .md file"));
        }
        Ok(Self(path.to_owned()))
    }

    /// The path as written in `SUMMARY.md`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The folder that holds this file, `""` at the book root.
    #[must_use]
    pub fn dir(&self) -> &str {
        self.0.rsplit_once('/').map_or("", |(dir, _)| dir)
    }

    /// The file name, such as `services.md`.
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.0.rsplit_once('/').map_or(self.0.as_str(), |(_, name)| name)
    }

    /// Whether this is a `README.md`, the landing page of a folder.
    #[must_use]
    pub fn is_readme(&self) -> bool {
        self.file_name() == README
    }

    /// The folder this section's children live in: `a/b` for both
    /// `a/b.md` and `a/b/README.md`.
    #[must_use]
    pub fn children_dir(&self) -> String {
        if self.is_readme() {
            self.dir().to_owned()
        } else {
            self.0.trim_end_matches(".md").to_owned()
        }
    }

    /// Joins a folder and a file name into a path.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::InvalidPath`] when the result is invalid.
    pub fn join(dir: &str, file: &str) -> Result<Self> {
        if dir.is_empty() {
            Self::parse(file)
        } else {
            Self::parse(&format!("{dir}/{file}"))
        }
    }

    /// The relative link from the folder of `self` to `target`, as mdBook
    /// expects in chapter text (`../other/page.md`).
    #[must_use]
    pub fn relative_to(&self, target: &str) -> String {
        relative(self.dir(), target)
    }
}

impl fmt::Display for ChapterPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for ChapterPath {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ChapterPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// The relative path from folder `from_dir` to `target` (both relative to
/// the same root, `/`-separated).
#[must_use]
pub fn relative(from_dir: &str, target: &str) -> String {
    let from: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let to: Vec<&str> = target.split('/').filter(|s| !s.is_empty()).collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let ups = from.len().saturating_sub(common);
    let mut parts: Vec<&str> = std::iter::repeat_n("..", ups).collect();
    parts.extend(to.iter().skip(common));
    parts.join("/")
}

/// Resolves a relative link `href` written in a file in folder `from_dir`.
/// Returns `None` when the link climbs above the root or is not relative.
#[must_use]
pub fn resolve(from_dir: &str, href: &str) -> Option<String> {
    if href.is_empty() || href.starts_with('/') || href.contains("://") || href.starts_with('#')
    {
        return None;
    }
    if href.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    }) {
        // mailto:, tel: and other schemes.
        return None;
    }
    let mut parts: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    for segment in href.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                let _ = parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Turns a title into a file-name slug: lower-case ASCII letters, digits
/// and single hyphens, never empty, at most 64 characters.
#[must_use]
pub fn slugify(title: &str) -> String {
    let mut slug = String::with_capacity(title.len());
    let mut hyphen = false;
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            hyphen = false;
        } else if !hyphen && !slug.is_empty() {
            slug.push('-');
            hyphen = true;
        }
        if slug.len() >= 64 {
            break;
        }
    }
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() || slug == "readme" || slug == "summary" {
        format!("section-{slug}").trim_end_matches('-').to_owned()
    } else {
        slug.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_book_paths() -> Result<()> {
        let path = ChapterPath::parse("architecture/services.md")?;
        assert_eq!(path.dir(), "architecture");
        assert_eq!(path.file_name(), "services.md");
        assert_eq!(path.children_dir(), "architecture/services");
        let readme = ChapterPath::parse("./architecture/README.md")?;
        assert_eq!(readme.as_str(), "architecture/README.md");
        assert!(readme.is_readme());
        assert_eq!(readme.children_dir(), "architecture");
        Ok(())
    }

    #[test]
    fn rejects_escapes() {
        for bad in [
            "",
            "/etc/passwd.md",
            "../secret.md",
            "a/../../b.md",
            "a//b.md",
            ".vellum/index.md",
            "a/.hidden.md",
            "c:/x.md",
            "a\\b.md",
            "notes.txt",
            "a/.md",
        ] {
            assert!(ChapterPath::parse(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn relative_links() {
        assert_eq!(relative("a/b", "a/c/d.md"), "../c/d.md");
        assert_eq!(relative("", "a/b.md"), "a/b.md");
        assert_eq!(relative("a", "b.md"), "../b.md");
        assert_eq!(resolve("a/b", "../c.md").as_deref(), Some("a/c.md"));
        assert_eq!(resolve("", "../c.md"), None);
        assert_eq!(resolve("", "https://northstar.dev"), None);
        assert_eq!(resolve("", "mailto:ops@northstar.dev"), None);
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Your first week!"), "your-first-week");
        assert_eq!(slugify("  ---  "), "section");
        assert_eq!(slugify("README"), "section-readme");
        assert_eq!(slugify("Café au lait"), "caf-au-lait");
    }
}
