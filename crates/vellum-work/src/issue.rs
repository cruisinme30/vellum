// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Issues: one Vellum Markdown file per issue (`teams/ENG/issues/ENG-142.md`).
//!
//! The front matter carries every field; the body is Vellum Markdown. Keys
//! this version does not know are kept and written back unchanged, so a
//! newer server or a hand edit never loses data.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::{Date, OffsetDateTime};

use crate::cycle::CycleKey;
use crate::error::WorkError;
use crate::id::{IssueId, MilestoneId, ProjectId, UserRef};
use crate::scale::{Estimate, Priority};
use crate::sla::Sla;
use crate::team::TeamKey;
use crate::workflow::{Category, StatusKey, Workflow};

/// An issue identifier: team key and number, written `ENG-142`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IssueKey {
    /// The team key.
    pub team: TeamKey,
    /// The number within the team, from 1.
    pub number: u64,
}

impl IssueKey {
    /// Parses `ENG-142`, in any case.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] if the text is not `KEY-n` with n ≥ 1.
    pub fn parse(text: &str) -> Result<Self, WorkError> {
        let invalid = || WorkError::Invalid {
            what: "issue identifier",
            value: text.to_owned(),
            reason: "must be KEY-n, such as ENG-142",
        };
        let (team, number) = text.rsplit_once('-').ok_or_else(invalid)?;
        let team = TeamKey::parse(team).map_err(|_| invalid())?;
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        let number: u64 = number.parse().map_err(|_| invalid())?;
        if number == 0 {
            return Err(invalid());
        }
        Ok(Self { team, number })
    }
}

impl fmt::Display for IssueKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.team, self.number)
    }
}

impl FromStr for IssueKey {
    type Err = WorkError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for IssueKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for IssueKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Self::parse(&text).map_err(de::Error::custom)
    }
}

/// The kinds of relation between two issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// This issue blocks the other.
    Blocks,
    /// This issue is blocked by the other.
    BlockedBy,
    /// The issues are related.
    Related,
    /// This issue duplicates the other.
    DuplicateOf,
}

impl RelationKind {
    /// The relation as seen from the other issue.
    #[must_use]
    pub const fn inverse(self) -> Option<Self> {
        match self {
            Self::Blocks => Some(Self::BlockedBy),
            Self::BlockedBy => Some(Self::Blocks),
            Self::Related => Some(Self::Related),
            Self::DuplicateOf => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Blocks => "blocks",
            Self::BlockedBy => "blocked_by",
            Self::Related => "related",
            Self::DuplicateOf => "duplicate_of",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        [
            Self::Blocks,
            Self::BlockedBy,
            Self::Related,
            Self::DuplicateOf,
        ]
        .into_iter()
        .find(|k| k.name() == name)
    }
}

/// A relation to another issue, written `{blocked_by: 01JB…}`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Relation {
    /// The kind of relation.
    pub kind: RelationKind,
    /// The other issue.
    pub issue: IssueId,
}

impl Serialize for Relation {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(1))?;
        map.serialize_entry(self.kind.name(), &self.issue)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for Relation {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Relation;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map with one key: blocks, blocked_by, related or duplicate_of")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Relation, A::Error> {
                let (name, issue): (String, IssueId) = map
                    .next_entry()?
                    .ok_or_else(|| de::Error::custom("empty relation"))?;
                if map.next_key::<de::IgnoredAny>()?.is_some() {
                    return Err(de::Error::custom("a relation has exactly one key"));
                }
                let kind = RelationKind::from_name(&name)
                    .ok_or_else(|| de::Error::custom(format!("unknown relation {name}")))?;
                Ok(Relation { kind, issue })
            }
        }
        d.deserialize_map(V)
    }
}

/// A link from an issue to something outside it: a pull request, a doc
/// block the issue was created from, a Sentry event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueLink {
    /// The target, such as `https://github.com/…/pull/7` or
    /// `vellum://doc/01JB…#^blk-3k9x2m7q`.
    pub url: String,
    /// A display title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// What the link is, such as `pull-request`, `source` or `sentry`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// Every field of an issue, exactly as stored in its file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    /// The immutable identity.
    pub id: IssueId,
    /// The owning team.
    pub team: TeamKey,
    /// The number within the team; with the team key, the identifier.
    pub number: u64,
    /// The title.
    pub title: String,
    /// The status key in the team's workflow.
    pub state: StatusKey,
    /// Priority, 0 to 4.
    #[serde(default)]
    pub priority: Priority,
    /// The estimate on the team's scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Estimate>,
    /// The assignee.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<UserRef>,
    /// Who created the issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator: Option<UserRef>,
    /// Label references (`gateway`, `type/bug`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// The project the issue belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectId>,
    /// The project milestone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub milestone: Option<MilestoneId>,
    /// The cycle, such as `2027-C04`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycle: Option<CycleKey>,
    /// The parent issue, for sub-issues.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<IssueId>,
    /// Relations to other issues.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relations: Vec<Relation>,
    /// People notified of changes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscribers: Vec<UserRef>,
    /// Identifiers the issue had before it moved team; they still resolve.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<IssueKey>,
    /// Links to pull requests, source blocks and other systems.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<IssueLink>,
    /// The due date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    /// The SLA clock, when an SLA rule applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sla: Option<Sla>,
    /// When the issue was created.
    #[serde(with = "crate::timestamp")]
    pub created_at: OffsetDateTime,
    /// When any field last changed.
    #[serde(with = "crate::timestamp")]
    pub updated_at: OffsetDateTime,
    /// When the issue first entered a started status.
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub started_at: Option<OffsetDateTime>,
    /// When the issue entered a completed status.
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub completed_at: Option<OffsetDateTime>,
    /// When the issue entered a canceled or duplicate status.
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub canceled_at: Option<OffsetDateTime>,
    /// When the issue left triage.
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub triaged_at: Option<OffsetDateTime>,
    /// Hidden from the triage inbox until this time.
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub snoozed_until: Option<OffsetDateTime>,
    /// The dialect version, always 1.
    #[serde(default = "one")]
    pub vellum: u8,
    /// Front matter keys this version does not know, kept verbatim.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
    /// The Vellum Markdown body.
    #[serde(skip)]
    pub body: String,
}

const fn one() -> u8 {
    1
}

impl Issue {
    /// A new issue with only the required fields set.
    #[must_use]
    pub fn new(
        id: IssueId,
        key: IssueKey,
        title: &str,
        state: StatusKey,
        now: OffsetDateTime,
    ) -> Self {
        Self {
            id,
            team: key.team,
            number: key.number,
            title: title.to_owned(),
            state,
            priority: Priority::None,
            estimate: None,
            assignee: None,
            creator: None,
            labels: Vec::new(),
            project: None,
            milestone: None,
            cycle: None,
            parent: None,
            relations: Vec::new(),
            subscribers: Vec::new(),
            aliases: Vec::new(),
            links: Vec::new(),
            due: None,
            sla: None,
            created_at: now,
            updated_at: now,
            started_at: None,
            completed_at: None,
            canceled_at: None,
            triaged_at: None,
            snoozed_until: None,
            vellum: 1,
            extra: BTreeMap::new(),
            body: String::new(),
        }
    }

    /// The current identifier, such as `ENG-142`.
    #[must_use]
    pub fn key(&self) -> IssueKey {
        IssueKey {
            team: self.team.clone(),
            number: self.number,
        }
    }

    /// The file path, relative to the workspace root.
    #[must_use]
    pub fn path(&self) -> String {
        path_for(&self.key())
    }

    /// The Git branch name for this issue: `user/eng-142-title-slug`.
    #[must_use]
    pub fn branch_name(&self, user: &UserRef) -> String {
        let key = self.key().to_string().to_ascii_lowercase();
        let slug = slug(&self.title, 40);
        if slug.is_empty() {
            format!("{user}/{key}")
        } else {
            format!("{user}/{key}-{slug}")
        }
    }

    /// The issues this one is blocked by.
    pub fn blockers(&self) -> impl Iterator<Item = &IssueId> {
        self.relations
            .iter()
            .filter(|r| r.kind == RelationKind::BlockedBy)
            .map(|r| &r.issue)
    }

    /// Moves the issue to `state`, keeping the lifecycle timestamps true.
    ///
    /// Entering started sets `started_at` once; entering completed sets
    /// `completed_at`; entering canceled or duplicate sets `canceled_at`;
    /// leaving triage sets `triaged_at` and clears `snoozed_until`. Leaving
    /// a closed category clears the matching close time, so reopening an
    /// issue is visible in insights. Returns the old and new categories.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownStatus`] if `state` is not in the
    /// workflow.
    pub fn set_state(
        &mut self,
        state: StatusKey,
        workflow: &Workflow,
        now: OffsetDateTime,
    ) -> Result<(Option<Category>, Category), WorkError> {
        let to = workflow
            .category_of(&state)
            .ok_or_else(|| WorkError::UnknownStatus {
                team: self.team.clone(),
                status: state.to_string(),
            })?;
        let from = workflow.category_of(&self.state);
        if from == Some(Category::Triage) && to != Category::Triage {
            self.triaged_at = Some(now);
            self.snoozed_until = None;
        }
        match to {
            Category::Started => {
                let _ = self.started_at.get_or_insert(now);
                self.completed_at = None;
                self.canceled_at = None;
            }
            Category::Completed => {
                let _ = self.started_at.get_or_insert(now);
                self.completed_at = Some(now);
                self.canceled_at = None;
            }
            Category::Canceled | Category::Duplicate => {
                self.canceled_at = Some(now);
                self.completed_at = None;
            }
            Category::Triage | Category::Backlog | Category::Unstarted => {
                self.completed_at = None;
                self.canceled_at = None;
            }
        }
        self.state = state;
        self.updated_at = now;
        Ok((from, to))
    }

    /// Parses an issue file.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if the front matter is missing or
    /// malformed, or if the file name does not match the identifier.
    pub fn from_markdown(path: &str, text: &str) -> Result<Self, WorkError> {
        let file_err = |reason: String| WorkError::File {
            path: path.to_owned(),
            reason,
        };
        let (front, body) = split_front_matter(text).map_err(|r| file_err(r.to_owned()))?;
        let mut issue: Self =
            serde_saphyr::from_str(front).map_err(|e| file_err(e.to_string()))?;
        if issue.number == 0 {
            return Err(file_err("number must be 1 or more".to_owned()));
        }
        let expected = format!("{}.md", issue.key());
        if !path.ends_with(&expected) {
            return Err(file_err(format!(
                "file name does not match {}",
                issue.key()
            )));
        }
        body.clone_into(&mut issue.body);
        Ok(issue)
    }

    /// Writes the issue as a Vellum Markdown file.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if the front matter cannot be serialised.
    pub fn to_markdown(&self) -> Result<String, WorkError> {
        let front = serde_saphyr::to_string(self).map_err(|e| WorkError::File {
            path: self.path(),
            reason: e.to_string(),
        })?;
        Ok(join_front_matter(&front, &self.body))
    }
}

/// The file path for an identifier: `teams/ENG/issues/ENG-142.md`.
#[must_use]
pub fn path_for(key: &IssueKey) -> String {
    format!("teams/{}/issues/{key}.md", key.team)
}

/// A lowercase ASCII slug of at most `max` characters, cut at a word.
#[must_use]
pub fn slug(text: &str, max: usize) -> String {
    let mut out = String::new();
    for word in text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let extra = usize::from(!out.is_empty()) + word.len();
        if out.len() + extra > max {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&word.to_ascii_lowercase());
    }
    out
}

/// Splits `---\n<yaml>\n---\n<body>` into front matter and body.
///
/// # Errors
///
/// Returns why the text has no front matter block.
pub fn split_front_matter(text: &str) -> Result<(&str, &str), &'static str> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .ok_or("the file does not start with front matter (---)")?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let front = rest.get(..offset).unwrap_or_default();
            let body = rest.get(offset + line.len()..).unwrap_or_default();
            return Ok((front, body.strip_prefix('\n').unwrap_or(body)));
        }
        offset += line.len();
    }
    Err("the front matter is not closed (---)")
}

/// Joins front matter YAML and a body into file text.
#[must_use]
pub fn join_front_matter(front: &str, body: &str) -> String {
    let mut out = String::with_capacity(front.len() + body.len() + 10);
    out.push_str("---\n");
    out.push_str(front);
    if !front.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n");
    if !body.is_empty() {
        out.push('\n');
        out.push_str(body);
        if !body.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::{date, datetime};

    fn workflow() -> Workflow {
        Workflow::standard().unwrap()
    }

    fn key(s: &str) -> StatusKey {
        StatusKey::parse(s).unwrap()
    }

    pub fn issue() -> Issue {
        Issue::new(
            "01JBZ3M8K2Q4R5S6T7V8W9X0A1".parse::<IssueId>().unwrap(),
            IssueKey::parse("ENG-142").unwrap(),
            "Replay 7 days of traffic in under 10 minutes",
            key("todo"),
            datetime!(2026-10-01 09:00 UTC),
        )
    }

    #[test]
    fn identifiers_parse_and_print() {
        let k = IssueKey::parse("eng-142").unwrap();
        assert_eq!(k.to_string(), "ENG-142");
        for bad in ["ENG", "ENG-", "ENG-0", "ENG-x", "-1", "E-1", "ENG-+1"] {
            assert!(IssueKey::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(path_for(&k), "teams/ENG/issues/ENG-142.md");
    }

    #[test]
    fn the_brief_example_parses() {
        let text = "---\n\
id: 01JBZ3M8K2Q4R5S6T7V8W9X0A1\nteam: ENG\nnumber: 142\n\
title: Replay 7 days of traffic in under 10 minutes\nstate: in-review\n\
priority: 1\nestimate: 5\nassignee: dev\nlabels: [gateway]\n\
project: 01JBZ3M8K2Q4R5S6T7V8W9X0P1\ncycle: 2027-C04\nparent: ~\n\
relations: [{blocked_by: 01JBZ3M8K2Q4R5S6T7V8W9X0A2}]\ndue: 2027-01-10\n\
sla: {type: urgent-24h, started_at: 2026-11-01T10:00Z, breaches_at: 2026-11-02T10:00Z}\n\
created_at: 2026-11-01T10:00:00Z\nupdated_at: 2026-11-01T10:00:00Z\n\
custom: kept\n---\n\nBody in Vellum Markdown.\n";
        let i = Issue::from_markdown("teams/ENG/issues/ENG-142.md", text).unwrap();
        assert_eq!(i.priority, Priority::Urgent);
        assert_eq!(i.estimate, Some(Estimate::Points(5)));
        assert_eq!(i.due, Some(date!(2027 - 01 - 10)));
        assert_eq!(i.relations[0].kind, RelationKind::BlockedBy);
        assert_eq!(i.cycle.as_ref().unwrap().as_str(), "2027-C04");
        assert_eq!(i.body, "Body in Vellum Markdown.\n");
        assert_eq!(i.extra.len(), 1);
        let again = Issue::from_markdown(&i.path(), &i.to_markdown().unwrap()).unwrap();
        assert_eq!(again, i);
        assert!(i.to_markdown().unwrap().contains("custom: kept"));
    }

    #[test]
    fn file_errors_are_reported() {
        let p = "teams/ENG/issues/ENG-142.md";
        assert!(Issue::from_markdown(p, "no front matter").is_err());
        assert!(Issue::from_markdown(p, "---\ntitle: x\n").is_err());
        let i = issue();
        let text = i.to_markdown().unwrap();
        assert!(Issue::from_markdown("teams/ENG/issues/ENG-9.md", &text).is_err());
        let bad_rel = text.replace("vellum: 1", "vellum: 1\nrelations: [{blocks: x}]");
        assert!(Issue::from_markdown(p, &bad_rel).is_err());
    }

    #[test]
    fn lifecycle_timestamps_follow_the_category() {
        let w = workflow();
        let mut i = issue();
        i.state = key("triage");
        i.snoozed_until = Some(datetime!(2026-10-03 00:00 UTC));
        let t1 = datetime!(2026-10-02 09:00 UTC);
        let (from, to) = i.set_state(key("in-progress"), &w, t1).unwrap();
        assert_eq!((from, to), (Some(Category::Triage), Category::Started));
        assert_eq!(i.triaged_at, Some(t1));
        assert_eq!(i.snoozed_until, None);
        assert_eq!(i.started_at, Some(t1));

        let t2 = datetime!(2026-10-05 09:00 UTC);
        let _ = i.set_state(key("in-review"), &w, t2).unwrap();
        assert_eq!(i.started_at, Some(t1));
        let _ = i.set_state(key("done"), &w, t2).unwrap();
        assert_eq!(i.completed_at, Some(t2));
        let _ = i.set_state(key("todo"), &w, t2).unwrap();
        assert_eq!(i.completed_at, None);
        let _ = i.set_state(key("duplicate"), &w, t2).unwrap();
        assert_eq!(i.canceled_at, Some(t2));
        assert!(i.set_state(key("nope"), &w, t2).is_err());
    }

    #[test]
    fn branch_names_follow_the_git_convention() {
        let i = issue();
        let dev = UserRef::parse("dev").unwrap();
        assert_eq!(
            i.branch_name(&dev),
            "dev/eng-142-replay-7-days-of-traffic-in-under-10"
        );
        assert_eq!(slug("!!!", 10), "");
    }

    #[test]
    fn front_matter_splits_with_and_without_body() {
        assert_eq!(
            split_front_matter("---\na: 1\n---\n").unwrap(),
            ("a: 1\n", "")
        );
        assert_eq!(
            split_front_matter("---\r\na: 1\r\n---\r\n\r\nx").unwrap().0,
            "a: 1\r\n"
        );
        assert_eq!(join_front_matter("a: 1", "x"), "---\na: 1\n---\n\nx\n");
    }
}
