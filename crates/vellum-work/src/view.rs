// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Views: list and board, grouping, ordering and saved views (R-WK-4).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::Duration;

use crate::error::WorkError;
use crate::filter::{Filter, Lookup};
use crate::graph::{Ctx, WorkGraph};
use crate::id::{IssueId, UserRef, ViewId};
use crate::issue::Issue;
use crate::scale::Priority;
use crate::team::TeamKey;
use crate::workflow::Category;

/// How a view lays issues out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// Rows, grouped into sections.
    #[default]
    List,
    /// Columns, one per group.
    Board,
}

/// The field issues are grouped by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    /// One group per status, in workflow order.
    #[default]
    Status,
    /// One group per assignee.
    Assignee,
    /// One group per priority, urgent first.
    Priority,
    /// One group per project.
    Project,
    /// One group per cycle.
    Cycle,
    /// One group per label; an issue shows in each of its labels.
    Label,
    /// One group per team.
    Team,
    /// No grouping.
    None,
}

/// The order of issues within a group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderBy {
    /// Urgent first, then most recently updated.
    #[default]
    Priority,
    /// Most recently updated first.
    Updated,
    /// Newest first.
    Created,
    /// Soonest due first; no due date last.
    Due,
    /// Highest estimate first.
    Estimate,
    /// By title.
    Title,
}

/// Which closed issues a view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowClosed {
    /// Every closed issue.
    All,
    /// Issues closed in the past 7 days.
    #[default]
    PastWeek,
    /// Issues closed in the past 30 days.
    PastMonth,
    /// No closed issues.
    None,
}

/// Who can open a saved view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    /// Only the owner.
    #[default]
    Private,
    /// Members of the view's team.
    Team,
    /// Everyone in the workspace.
    Workspace,
}

/// A saved view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct View {
    /// The view ID.
    pub id: ViewId,
    /// The name.
    pub name: String,
    /// Who made it.
    pub owner: UserRef,
    /// The team the view is scoped to; `None` spans every team.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamKey>,
    /// Who can open it.
    #[serde(default)]
    pub visibility: Visibility,
    /// List or board.
    #[serde(default)]
    pub layout: Layout,
    /// Which issues.
    #[serde(default)]
    pub filter: Filter,
    /// Grouping.
    #[serde(default)]
    pub group_by: GroupBy,
    /// Ordering within groups.
    #[serde(default)]
    pub order_by: OrderBy,
    /// Which closed issues to show.
    #[serde(default)]
    pub show_closed: ShowClosed,
    /// Whether sub-issues show alongside their parents.
    #[serde(default = "yes")]
    pub show_sub_issues: bool,
}

const fn yes() -> bool {
    true
}

impl View {
    /// Checks the filter and that a team-visible view names a team.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] naming the problem.
    pub fn check(&self) -> Result<(), WorkError> {
        let invalid = |reason: String| WorkError::Invalid {
            what: "view",
            value: self.name.clone(),
            reason: Box::leak(reason.into_boxed_str()),
        };
        if self.visibility == Visibility::Team && self.team.is_none() {
            return Err(invalid("a team-visible view needs a team".to_owned()));
        }
        self.filter.check().map_err(invalid)
    }

    /// Whether `user` may open the view; `member_of` says whether the user
    /// belongs to a team.
    #[must_use]
    pub fn visible_to(&self, user: &UserRef, member_of: impl Fn(&TeamKey) -> bool) -> bool {
        match self.visibility {
            Visibility::Private => &self.owner == user,
            Visibility::Workspace => true,
            Visibility::Team => &self.owner == user || self.team.as_ref().is_some_and(member_of),
        }
    }

    /// Where the view is stored: `views/<id>.yaml`.
    #[must_use]
    pub fn path(&self) -> String {
        format!("views/{}.yaml", self.id)
    }

    /// Parses a stored view and checks it.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] for bad YAML, or what [`View::check`]
    /// finds.
    pub fn from_yaml(path: &str, text: &str) -> Result<Self, WorkError> {
        let view: Self = serde_saphyr::from_str(text).map_err(|e| WorkError::File {
            path: path.to_owned(),
            reason: e.to_string(),
        })?;
        view.check()?;
        Ok(view)
    }

    /// Serialises the view to its file's text.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if the view cannot be written as YAML.
    pub fn to_yaml(&self) -> Result<String, WorkError> {
        serde_saphyr::to_string(self).map_err(|e| WorkError::File {
            path: self.path(),
            reason: e.to_string(),
        })
    }
}

/// One group of a view's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// The group's value as filter text (`in-review`, `urgent`, `dev`);
    /// `None` for the "no value" group.
    pub key: Option<String>,
    /// The issues, in order.
    pub issues: Vec<IssueId>,
}

/// Runs a view for a viewer. Board views of a single team list every
/// status (or priority) column, even when empty.
#[must_use]
pub fn run(view: &View, graph: &WorkGraph, ctx: &Ctx, me: Option<&UserRef>) -> Vec<Group> {
    let ectx = graph.eval_context(ctx, me);
    let closed_since = match view.show_closed {
        ShowClosed::All => None,
        ShowClosed::PastWeek => Some(ctx.now - Duration::days(7)),
        ShowClosed::PastMonth => Some(ctx.now - Duration::days(30)),
        ShowClosed::None => Some(ctx.now + Duration::days(1)),
    };
    let mut issues: Vec<&Issue> = graph
        .issues()
        .filter(|i| view.team.as_ref().is_none_or(|t| &i.team == t))
        .filter(|i| view.show_sub_issues || i.parent.is_none())
        .filter(|i| {
            let closed = graph.category_of(i).is_some_and(Category::is_closed);
            let at = i.completed_at.or(i.canceled_at).unwrap_or(i.updated_at);
            !closed || closed_since.is_none_or(|since| at >= since)
        })
        .filter(|i| view.filter.matches(i, &ectx))
        .collect();
    sort(&mut issues, view.order_by);

    let mut groups: BTreeMap<(u32, String), Group> = BTreeMap::new();
    if view.layout == Layout::Board {
        for (rank, key) in fixed_columns(view, graph) {
            let _ = groups.insert(
                (rank, key.clone()),
                Group {
                    key: Some(key),
                    issues: Vec::new(),
                },
            );
        }
    }
    for issue in issues {
        for (rank, key) in group_keys(view.group_by, issue, graph) {
            let sort_key = (rank, key.clone().unwrap_or_default());
            groups
                .entry(sort_key)
                .or_insert_with(|| Group {
                    key,
                    issues: Vec::new(),
                })
                .issues
                .push(issue.id);
        }
    }
    groups.into_values().collect()
}

/// Rank for the "no value" group: after every real value.
const LAST: u32 = u32::MAX;

fn fixed_columns(view: &View, graph: &WorkGraph) -> Vec<(u32, String)> {
    match view.group_by {
        GroupBy::Status => view
            .team
            .as_ref()
            .and_then(|t| graph.team(t))
            .map(|t| {
                t.workflow
                    .board_order()
                    .into_iter()
                    .filter(|s| {
                        s.category != Category::Duplicate || view.show_closed == ShowClosed::All
                    })
                    .map(|s| (status_rank(t.workflow.position(&s.key)), s.key.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
        GroupBy::Priority => Priority::ALL
            .into_iter()
            .map(|p| (u32::from(p.rank()), p.name().to_owned()))
            .collect(),
        _ => Vec::new(),
    }
}

fn status_rank(position: Option<(Category, usize)>) -> u32 {
    position.map_or(LAST, |(c, i)| {
        let cat = Category::ALL.iter().position(|x| *x == c).unwrap_or(0);
        u32::try_from(cat * 1000 + i).unwrap_or(LAST)
    })
}

fn group_keys(by: GroupBy, issue: &Issue, graph: &WorkGraph) -> Vec<(u32, Option<String>)> {
    let named = |v: Option<String>| v.map_or_else(|| vec![(LAST, None)], |v| vec![(0, Some(v))]);
    match by {
        GroupBy::Status => {
            let pos = graph
                .team(&issue.team)
                .and_then(|t| t.workflow.position(&issue.state));
            vec![(status_rank(pos), Some(issue.state.to_string()))]
        }
        GroupBy::Priority => vec![(
            u32::from(issue.priority.rank()),
            Some(issue.priority.name().to_owned()),
        )],
        GroupBy::Assignee => named(issue.assignee.as_ref().map(ToString::to_string)),
        GroupBy::Project => named(issue.project.as_ref().map(|p| {
            graph
                .project(p)
                .map_or_else(|| p.to_string(), |x| x.title.clone())
        })),
        GroupBy::Cycle => named(issue.cycle.as_ref().map(ToString::to_string)),
        GroupBy::Team => vec![(0, Some(issue.team.to_string()))],
        GroupBy::Label if issue.labels.is_empty() => vec![(LAST, None)],
        GroupBy::Label => issue.labels.iter().map(|l| (0, Some(l.clone()))).collect(),
        GroupBy::None => vec![(0, None)],
    }
}

/// Sorts issues in place by `order`, ties broken by most recent update.
pub fn sort(issues: &mut [&Issue], order: OrderBy) {
    issues.sort_by(|a, b| {
        let primary = match order {
            OrderBy::Priority => a.priority.rank().cmp(&b.priority.rank()),
            OrderBy::Updated => std::cmp::Ordering::Equal,
            OrderBy::Created => b.created_at.cmp(&a.created_at),
            OrderBy::Due => match (a.due, b.due) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            },
            OrderBy::Estimate => b
                .estimate
                .map(super::scale::Estimate::points)
                .cmp(&a.estimate.map(super::scale::Estimate::points)),
            OrderBy::Title => a.title.to_lowercase().cmp(&b.title.to_lowercase()),
        };
        primary
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Whether the issue's blockers include an open issue.
#[must_use]
pub fn is_blocked(issue: &Issue, graph: &WorkGraph) -> bool {
    issue.blockers().any(|b| graph.is_open(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_view_round_trips_through_its_file() {
        let filter: Filter = serde_json::from_str(
            r#"{"and":[{"field":"priority","op":"is","values":["urgent","high"]},
                {"or":[{"field":"assignee","op":"is","values":["@me"]},
                       {"field":"label","op":"includes_all","values":["gateway"]}]}]}"#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let view = View {
            id: ViewId::generate(),
            name: "Urgent gateway".to_owned(),
            owner: UserRef::parse("dev").unwrap_or_else(|e| panic!("{e}")),
            team: Some(TeamKey::parse("ENG").unwrap_or_else(|e| panic!("{e}"))),
            visibility: Visibility::Team,
            layout: Layout::Board,
            filter,
            group_by: GroupBy::Assignee,
            order_by: OrderBy::Due,
            show_closed: ShowClosed::None,
            show_sub_issues: false,
        };
        let text = view.to_yaml().unwrap_or_else(|e| panic!("{e}"));
        let back = View::from_yaml(&view.path(), &text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back, view);
        let path = view.path();
        assert!(path.starts_with("views/"));
        assert_eq!(
            std::path::Path::new(&path)
                .extension()
                .and_then(|e| e.to_str()),
            Some("yaml")
        );
    }

    #[test]
    fn a_team_view_without_a_team_is_refused() {
        let text = "id: 01JBV0000000000000000000AA\nname: x\nowner: dev\nvisibility: team\n";
        assert!(View::from_yaml("views/x.yaml", text).is_err());
    }
}
