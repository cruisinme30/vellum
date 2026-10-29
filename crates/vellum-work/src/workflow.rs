// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Custom statuses in fixed categories (D15).
//!
//! A team names its statuses freely; every view, automation and metric keys
//! off the status's [`Category`], which is one of seven fixed values. There
//! are no transition rules: any status can move to any other.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

use crate::error::WorkError;

/// The seven fixed status categories.
///
/// The declaration order is the board and focus order of the categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    /// New issues waiting for a team member to accept them.
    Triage,
    /// Accepted but not planned.
    Backlog,
    /// Planned, not started.
    Unstarted,
    /// Being worked on (in progress, in review, ready to merge).
    Started,
    /// Done.
    Completed,
    /// Will not be done.
    Canceled,
    /// Closed as a duplicate of another issue.
    Duplicate,
}

impl Category {
    /// Every category, in board order.
    pub const ALL: [Self; 7] = [
        Self::Triage,
        Self::Backlog,
        Self::Unstarted,
        Self::Started,
        Self::Completed,
        Self::Canceled,
        Self::Duplicate,
    ];

    /// The name used in files and filters.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Triage => "triage",
            Self::Backlog => "backlog",
            Self::Unstarted => "unstarted",
            Self::Started => "started",
            Self::Completed => "completed",
            Self::Canceled => "canceled",
            Self::Duplicate => "duplicate",
        }
    }

    /// Parses a category name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == name)
    }

    /// Whether work on the issue is over: completed, canceled or duplicate.
    #[must_use]
    pub const fn is_closed(self) -> bool {
        matches!(self, Self::Completed | Self::Canceled | Self::Duplicate)
    }

    /// Whether the issue is accepted and still open: unstarted or started.
    #[must_use]
    pub const fn is_active(self) -> bool {
        matches!(self, Self::Unstarted | Self::Started)
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The stable key of a status, written in issue files (`state: in-review`).
///
/// Keys are lowercase slugs: letters, digits and single dashes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct StatusKey(String);

impl StatusKey {
    /// Parses a status key.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] unless the key is a lowercase slug of
    /// 1 to 40 characters.
    pub fn parse(text: &str) -> Result<Self, WorkError> {
        let invalid = |reason| WorkError::Invalid {
            what: "status key",
            value: text.to_owned(),
            reason,
        };
        if text.is_empty() || text.len() > 40 {
            return Err(invalid("must be 1 to 40 characters"));
        }
        if !text
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            || text.starts_with('-')
            || text.ends_with('-')
            || text.contains("--")
        {
            return Err(invalid("must be a lowercase slug such as in-review"));
        }
        Ok(Self(text.to_owned()))
    }

    /// The key as written in files.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StatusKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for StatusKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// One status in a team's workflow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Status {
    /// The stable key written in issue files.
    pub key: StatusKey,
    /// The display name, free text.
    pub name: String,
    /// The fixed category this status belongs to.
    pub category: Category,
    /// An optional display colour, such as `#4F5B92`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl Status {
    fn new(key: &str, name: &str, category: Category) -> Result<Self, WorkError> {
        Ok(Self {
            key: StatusKey::parse(key)?,
            name: name.to_owned(),
            category,
            color: None,
        })
    }
}

/// A team's ordered list of statuses and its default status (R-WK-3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workflow {
    /// The statuses, in display order within each category.
    pub statuses: Vec<Status>,
    /// The status new accepted issues start in; backlog or unstarted.
    pub default: StatusKey,
}

impl Workflow {
    /// The workflow every new team starts with: Triage, Backlog, Todo,
    /// In Progress, In Review, Ready to merge, Done, Canceled, Duplicate.
    ///
    /// # Errors
    ///
    /// Never in practice; the keys are fixed and valid.
    pub fn standard() -> Result<Self, WorkError> {
        use Category as C;
        Ok(Self {
            statuses: vec![
                Status::new("triage", "Triage", C::Triage)?,
                Status::new("backlog", "Backlog", C::Backlog)?,
                Status::new("todo", "Todo", C::Unstarted)?,
                Status::new("in-progress", "In Progress", C::Started)?,
                Status::new("in-review", "In Review", C::Started)?,
                Status::new("ready-to-merge", "Ready to merge", C::Started)?,
                Status::new("done", "Done", C::Completed)?,
                Status::new("canceled", "Canceled", C::Canceled)?,
                Status::new("duplicate", "Duplicate", C::Duplicate)?,
            ],
            default: StatusKey::parse("backlog")?,
        })
    }

    /// Checks the workflow's rules, returning the first broken one.
    ///
    /// The rules: keys are unique; there is exactly one duplicate status
    /// and at most one triage status; there is at least one status in each
    /// of backlog-or-unstarted, started, completed and canceled; and the
    /// default status exists and is backlog or unstarted.
    ///
    /// # Errors
    ///
    /// Returns the broken rule as text.
    pub fn check(&self) -> Result<(), String> {
        for (i, s) in self.statuses.iter().enumerate() {
            if self.statuses.iter().skip(i + 1).any(|t| t.key == s.key) {
                return Err(format!("status key {} is used twice", s.key));
            }
        }
        let count = |c: Category| self.statuses.iter().filter(|s| s.category == c).count();
        if count(Category::Duplicate) != 1 {
            return Err("needs exactly one duplicate status".to_owned());
        }
        if count(Category::Triage) > 1 {
            return Err("has more than one triage status".to_owned());
        }
        if count(Category::Backlog) + count(Category::Unstarted) == 0 {
            return Err("needs a backlog or unstarted status".to_owned());
        }
        for c in [Category::Started, Category::Completed, Category::Canceled] {
            if count(c) == 0 {
                return Err(format!("needs a {c} status"));
            }
        }
        match self.get(&self.default) {
            None => Err(format!("default status {} does not exist", self.default)),
            Some(s) if !matches!(s.category, Category::Backlog | Category::Unstarted) => {
                Err("default status must be backlog or unstarted".to_owned())
            }
            Some(_) => Ok(()),
        }
    }

    /// The status with this key.
    #[must_use]
    pub fn get(&self, key: &StatusKey) -> Option<&Status> {
        self.statuses.iter().find(|s| &s.key == key)
    }

    /// The category of the status with this key.
    #[must_use]
    pub fn category_of(&self, key: &StatusKey) -> Option<Category> {
        self.get(key).map(|s| s.category)
    }

    /// The first status of a category, in display order.
    #[must_use]
    pub fn first_in(&self, category: Category) -> Option<&Status> {
        self.statuses.iter().find(|s| s.category == category)
    }

    /// The board position of a status: category first, then list order.
    #[must_use]
    pub fn position(&self, key: &StatusKey) -> Option<(Category, usize)> {
        self.statuses
            .iter()
            .position(|s| &s.key == key)
            .and_then(|i| self.statuses.get(i).map(|s| (s.category, i)))
    }

    /// The statuses in board order: by category, then list order.
    #[must_use]
    pub fn board_order(&self) -> Vec<&Status> {
        let mut out: Vec<&Status> = self.statuses.iter().collect();
        out.sort_by_key(|s| s.category);
        out
    }

    /// The status this workflow uses for an issue arriving from another
    /// team in `key`'s place: a status with the same key if its category
    /// matches, else the first status of the same category, else the
    /// default status.
    #[must_use]
    pub fn map_from(&self, key: &StatusKey, category: Category) -> StatusKey {
        if self.category_of(key) == Some(category) {
            return key.clone();
        }
        self.first_in(category)
            .map_or(&self.default, |s| &s.key)
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> StatusKey {
        StatusKey::parse(s).unwrap()
    }

    #[test]
    fn the_standard_workflow_is_valid() {
        let w = Workflow::standard().unwrap();
        w.check().unwrap();
        assert_eq!(w.category_of(&key("in-review")), Some(Category::Started));
        assert_eq!(w.first_in(Category::Completed).unwrap().key, key("done"));
    }

    #[test]
    fn workflow_rules_are_enforced() {
        let mut w = Workflow::standard().unwrap();
        w.default = key("done");
        assert!(w.check().unwrap_err().contains("backlog or unstarted"));

        let mut w = Workflow::standard().unwrap();
        w.statuses.retain(|s| s.category != Category::Duplicate);
        assert!(w.check().unwrap_err().contains("duplicate"));

        let mut w = Workflow::standard().unwrap();
        let extra = w.statuses[0].clone();
        w.statuses.push(extra);
        assert!(w.check().unwrap_err().contains("used twice"));

        let mut w = Workflow::standard().unwrap();
        w.statuses.retain(|s| s.category != Category::Canceled);
        assert!(w.check().unwrap_err().contains("canceled"));
    }

    #[test]
    fn status_keys_are_slugs() {
        assert!(StatusKey::parse("in-review").is_ok());
        for bad in ["", "In Review", "-x", "x-", "a--b", "a_b"] {
            assert!(StatusKey::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn mapping_keeps_the_category() {
        let w = Workflow::standard().unwrap();
        assert_eq!(
            w.map_from(&key("doing"), Category::Started),
            key("in-progress")
        );
        assert_eq!(
            w.map_from(&key("in-review"), Category::Started),
            key("in-review")
        );
        assert_eq!(w.map_from(&key("x"), Category::Triage), key("triage"));
    }

    #[test]
    fn board_order_groups_by_category() {
        let mut w = Workflow::standard().unwrap();
        w.statuses.reverse();
        let order: Vec<_> = w.board_order().iter().map(|s| s.key.as_str()).collect();
        assert_eq!(order.first(), Some(&"triage"));
        assert_eq!(order.last(), Some(&"duplicate"));
    }
}
