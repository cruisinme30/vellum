// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `book.yaml`: the Vellum settings of a book.
//!
//! `SUMMARY.md` holds order and nesting; `book.yaml` holds everything mdBook
//! has no place for: the book id, numbering style, maximum depth, export
//! profile, and per-section owners, reviewers and access restrictions,
//! keyed by section id.
//!
//! ```yaml
//! vellum: 1
//! id: 01JBK00000000000000000BOOK
//! title: Northstar handbook
//! numbering: decimal
//! max_depth: 3
//! nodes:
//!   01JBK0000000000000000000A1:
//!     path: onboarding/README.md
//!     owners: [user:maya]
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{BookError, Result};
use crate::id::{BookId, NodeId};
use crate::path::ChapterPath;

/// The file name of the manifest inside a book folder.
pub const FILE_NAME: &str = "book.yaml";

/// The default and maximum nesting depth (R-BK-1).
pub const DEFAULT_MAX_DEPTH: u8 = 3;
/// The deepest a book may be configured to nest.
pub const LIMIT_MAX_DEPTH: u8 = 6;

/// How section numbers are written. Numbers are always computed from the
/// tree, never stored (R-BK-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NumberingStyle {
    /// `1`, `1.2`, `1.2.3`.
    #[default]
    Decimal,
    /// `I`, `I.A`, `I.A.1`, `I.A.1.a`, `I.A.1.a.i`, `I.A.1.a.i.1`.
    Outline,
    /// No numbers; cross-references show the title only.
    None,
}

impl NumberingStyle {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Decimal => "decimal",
            Self::Outline => "outline",
            Self::None => "none",
        }
    }
}

impl Serialize for NumberingStyle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NumberingStyle {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        match raw.trim() {
            "decimal" | "1.1" => Ok(Self::Decimal),
            "outline" | "I.A.1" => Ok(Self::Outline),
            "none" => Ok(Self::None),
            other => Err(serde::de::Error::custom(format!(
                "unknown numbering {other:?}; use decimal, outline or none"
            ))),
        }
    }
}

/// Someone a section can be restricted to, owned by or reviewed by:
/// `user:<name>` or `group:<name>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Principal(String);

impl Principal {
    /// Validates a principal.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::InvalidPrincipal`] for anything other than
    /// `user:<name>` or `group:<name>` with a non-empty name free of
    /// whitespace and control characters.
    pub fn parse(value: &str) -> Result<Self> {
        let ok = value.split_once(':').is_some_and(|(kind, name)| {
            matches!(kind, "user" | "group")
                && !name.is_empty()
                && name.len() <= 256
                && !name.chars().any(|c| c.is_whitespace() || c.is_control())
        });
        if ok {
            Ok(Self(value.to_owned()))
        } else {
            Err(BookError::InvalidPrincipal(value.to_owned()))
        }
    }

    /// The principal as written.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Principal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Principal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Principal {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// The Vellum settings of one section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeEntry {
    /// Where the section's file lives, relative to the book folder.
    pub path: ChapterPath,
    /// Section owners; empty means inherited from the enclosing section.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub owners: BTreeSet<Principal>,
    /// Section reviewers; empty means inherited from the enclosing section.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub reviewers: BTreeSet<Principal>,
    /// When set, only these principals (among those the book already
    /// admits) can see the section and everything under it (R-BK-9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restrict_to: Option<BTreeSet<Principal>>,
}

impl NodeEntry {
    /// An entry with no owners, reviewers or restriction.
    #[must_use]
    pub const fn new(path: ChapterPath) -> Self {
        Self {
            path,
            owners: BTreeSet::new(),
            reviewers: BTreeSet::new(),
            restrict_to: None,
        }
    }
}

const fn default_max_depth() -> u8 {
    DEFAULT_MAX_DEPTH
}

const fn format_version() -> u32 {
    1
}

/// The parsed `book.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The format version; always 1.
    #[serde(default = "format_version")]
    pub vellum: u32,
    /// The book's identity.
    pub id: BookId,
    /// The book's title.
    pub title: String,
    /// How sections are numbered.
    #[serde(default)]
    pub numbering: NumberingStyle,
    /// How deep sections may nest, 1 to 6.
    #[serde(default = "default_max_depth")]
    pub max_depth: u8,
    /// The export profile used by default; owned by the export service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_profile: Option<String>,
    /// Per-section settings keyed by section id.
    #[serde(default)]
    pub nodes: BTreeMap<NodeId, NodeEntry>,
}

impl Manifest {
    /// A manifest for a new, empty book.
    #[must_use]
    pub const fn new(id: BookId, title: String) -> Self {
        Self {
            vellum: 1,
            id,
            title,
            numbering: NumberingStyle::Decimal,
            max_depth: DEFAULT_MAX_DEPTH,
            export_profile: None,
            nodes: BTreeMap::new(),
        }
    }

    /// Parses and validates `book.yaml`.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::Manifest`] for malformed YAML, unknown fields,
    /// an unsupported version, a `max_depth` outside 1 to 6, or two
    /// sections sharing a path.
    pub fn parse(text: &str) -> Result<Self> {
        let manifest: Self =
            serde_yaml_ng::from_str(text).map_err(|e| BookError::Manifest(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Checks the invariants [`Manifest::parse`] enforces.
    ///
    /// # Errors
    ///
    /// See [`Manifest::parse`].
    pub fn validate(&self) -> Result<()> {
        if self.vellum != 1 {
            return Err(BookError::Manifest(format!(
                "unsupported format version {}",
                self.vellum
            )));
        }
        if !(1..=LIMIT_MAX_DEPTH).contains(&self.max_depth) {
            return Err(BookError::Manifest(format!(
                "max_depth must be 1 to {LIMIT_MAX_DEPTH}, not {}",
                self.max_depth
            )));
        }
        if self.title.trim().is_empty() {
            return Err(BookError::Manifest("title must not be empty".to_owned()));
        }
        let mut seen = BTreeSet::new();
        for (id, entry) in &self.nodes {
            if !seen.insert(&entry.path) {
                return Err(BookError::Manifest(format!(
                    "{id} shares the path {} with another section",
                    entry.path
                )));
            }
        }
        Ok(())
    }

    /// Serialises the manifest as YAML.
    ///
    /// # Errors
    ///
    /// Returns [`BookError::Manifest`] if serialisation fails.
    pub fn to_yaml(&self) -> Result<String> {
        serde_yaml_ng::to_string(self).map_err(|e| BookError::Manifest(e.to_string()))
    }

    /// The id of the section stored at `path`.
    #[must_use]
    pub fn id_at(&self, path: &ChapterPath) -> Option<&NodeId> {
        self.nodes
            .iter()
            .find_map(|(id, entry)| (&entry.path == path).then_some(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
vellum: 1
id: 01JBK00000000000000000B00K
title: Northstar handbook
numbering: \"1.1\"
max_depth: 4
nodes:
  01JBK0000000000000000000A1:
    path: onboarding/README.md
    owners: [user:maya]
    restrict_to: [group:staff]
";

    #[test]
    fn parses_the_prd_example() -> Result<()> {
        let manifest = Manifest::parse(SAMPLE)?;
        assert_eq!(manifest.numbering, NumberingStyle::Decimal);
        assert_eq!(manifest.max_depth, 4);
        let id = NodeId::parse("01JBK0000000000000000000A1")?;
        let entry = manifest
            .nodes
            .get(&id)
            .ok_or_else(|| BookError::UnknownNode(id.clone()))?;
        assert_eq!(entry.path.as_str(), "onboarding/README.md");
        assert_eq!(manifest.id_at(&entry.path), Some(&id));
        Ok(())
    }

    #[test]
    fn round_trips() -> Result<()> {
        let manifest = Manifest::parse(SAMPLE)?;
        let again = Manifest::parse(&manifest.to_yaml()?)?;
        assert_eq!(manifest, again);
        assert!(manifest.to_yaml()?.contains("numbering: decimal"));
        Ok(())
    }

    #[test]
    fn rejects_bad_manifests() {
        let bad_depth = SAMPLE.replace("max_depth: 4", "max_depth: 7");
        assert!(Manifest::parse(&bad_depth).is_err());
        let unknown = format!("{SAMPLE}colour: blue\n");
        assert!(Manifest::parse(&unknown).is_err());
        let escape = SAMPLE.replace("onboarding/README.md", "../x.md");
        assert!(Manifest::parse(&escape).is_err());
        let principal = SAMPLE.replace("user:maya", "maya");
        assert!(Manifest::parse(&principal).is_err());
    }
}
