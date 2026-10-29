// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! My Issues: assigned, created, subscribed and activity, in focus order
//! (R-WK-5).
//!
//! Focus order puts what needs attention first: urgent, SLA-bound,
//! blocked, in the current cycle, other active, triage, backlog, done.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::filter::Lookup;
use crate::graph::{Ctx, WorkGraph};
use crate::id::{IssueId, UserRef};
use crate::issue::Issue;
use crate::scale::Priority;
use crate::view::is_blocked;
use crate::workflow::Category;

/// The My Issues tabs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tab {
    /// Assigned to me.
    Assigned,
    /// Created by me.
    Created,
    /// I am subscribed.
    Subscribed,
    /// Issues I recently acted on, newest first.
    Activity,
}

/// The focus buckets, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Focus {
    /// Urgent priority.
    Urgent,
    /// A running SLA clock, soonest breach first.
    Sla,
    /// Blocked by an open issue.
    Blocked,
    /// In the team's current cycle.
    Cycle,
    /// Other unstarted or started issues.
    Active,
    /// In triage.
    Triage,
    /// In the backlog.
    Backlog,
    /// Completed, canceled or duplicate.
    Done,
}

/// The focus bucket an issue belongs to.
#[must_use]
pub fn focus(issue: &Issue, graph: &WorkGraph, ctx: &Ctx) -> Focus {
    let category = graph.category_of(issue).unwrap_or(Category::Backlog);
    if category.is_closed() {
        Focus::Done
    } else if issue.priority == Priority::Urgent {
        Focus::Urgent
    } else if issue.sla.as_ref().is_some_and(crate::sla::Sla::is_running) {
        Focus::Sla
    } else if is_blocked(issue, graph) {
        Focus::Blocked
    } else if issue.cycle.is_some() && issue.cycle == graph.current_cycle(&issue.team, ctx.today) {
        Focus::Cycle
    } else if category.is_active() {
        Focus::Active
    } else if category == Category::Triage {
        Focus::Triage
    } else {
        Focus::Backlog
    }
}

/// My issues for a tab, in focus order (or, for Activity, by `activity`:
/// my latest action time per issue, from the event log).
#[must_use]
pub fn my_issues<'g>(
    graph: &'g WorkGraph,
    me: &UserRef,
    tab: Tab,
    ctx: &Ctx,
    activity: &[(IssueId, OffsetDateTime)],
) -> Vec<(Focus, &'g Issue)> {
    if tab == Tab::Activity {
        let mut list: Vec<(OffsetDateTime, &Issue)> = activity
            .iter()
            .filter_map(|(id, at)| graph.issue(id).map(|i| (*at, i)))
            .collect();
        list.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
        return list
            .into_iter()
            .map(|(_, i)| (focus(i, graph, ctx), i))
            .collect();
    }
    let mut list: Vec<(Focus, &Issue)> = graph
        .issues()
        .filter(|i| match tab {
            Tab::Assigned => i.assignee.as_ref() == Some(me),
            Tab::Created => i.creator.as_ref() == Some(me),
            Tab::Subscribed => i.subscribers.contains(me),
            Tab::Activity => false,
        })
        .map(|i| (focus(i, graph, ctx), i))
        .collect();
    list.sort_by(|(fa, a), (fb, b)| {
        fa.cmp(fb)
            .then_with(|| match fa {
                Focus::Sla => {
                    let due = |i: &Issue| i.sla.as_ref().map(|s| s.breaches_at);
                    due(a).cmp(&due(b))
                }
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| a.priority.rank().cmp(&b.priority.rank()))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    list
}
