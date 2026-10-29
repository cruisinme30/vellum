// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Labels. Issues name labels in front matter (`labels: [gateway]`).

use serde::{Deserialize, Serialize};

/// A label, owned by a team or by the whole workspace.
///
/// A label in a group is written `group/name` in issue files, and an issue
/// carries at most one label per group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Label {
    /// The label name, unique within its owner (case-insensitive).
    pub name: String,
    /// The group the label belongs to, if any, such as `type`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// A display colour, such as `#E5484D`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// What the label means.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Label {
    /// The label as written in issue files: `name` or `group/name`.
    #[must_use]
    pub fn reference(&self) -> String {
        self.group
            .as_ref()
            .map_or_else(|| self.name.clone(), |g| format!("{g}/{}", self.name))
    }

    /// Whether two labels have the same reference, ignoring case.
    #[must_use]
    pub fn same_name(&self, other: &Self) -> bool {
        self.reference().eq_ignore_ascii_case(&other.reference())
    }
}

/// The group part of a label reference (`type` in `type/bug`).
#[must_use]
pub fn group_of(reference: &str) -> Option<&str> {
    reference.split_once('/').map(|(g, _)| g)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_include_the_group() {
        let l = Label {
            name: "bug".into(),
            group: Some("type".into()),
            color: None,
            description: None,
        };
        assert_eq!(l.reference(), "type/bug");
        assert_eq!(group_of("type/bug"), Some("type"));
        assert_eq!(group_of("gateway"), None);
    }
}
