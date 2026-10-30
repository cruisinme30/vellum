// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! SLAs: rules that put a deadline on an issue, and the risk colours.
//!
//! Rules are evaluated on every create and update; the first matching rule
//! sets the clock. Defaults: urgent issues within 24 hours, high within one
//! week. Risk moves gray → yellow (half the time used) → orange (three
//! quarters) → red (breached). Each step up is reported once, for the
//! notifications service (D77).

use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::filter::{Condition, EvalContext, Field, Filter, Op};
use crate::id::UserRef;
use crate::issue::Issue;
use crate::workflow::Category;

/// How close an SLA is to breaching.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    /// Less than half the time used.
    Gray,
    /// At least half the time used.
    Yellow,
    /// At least three quarters of the time used.
    Orange,
    /// Breached.
    Red,
}

impl Risk {
    /// The colour name used in filters and files.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gray => "gray",
            Self::Yellow => "yellow",
            Self::Orange => "orange",
            Self::Red => "red",
        }
    }

    /// Parses a colour name.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Gray, Self::Yellow, Self::Orange, Self::Red]
            .into_iter()
            .find(|r| r.name() == name)
    }
}

/// The SLA clock on an issue, written in front matter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sla {
    /// The rule that set the clock, such as `urgent-24h`.
    #[serde(rename = "type")]
    pub kind: String,
    /// When the clock started.
    #[serde(with = "crate::timestamp")]
    pub started_at: OffsetDateTime,
    /// When the SLA breaches.
    #[serde(with = "crate::timestamp")]
    pub breaches_at: OffsetDateTime,
    /// When the issue closed and the clock stopped.
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub stopped_at: Option<OffsetDateTime>,
    /// The highest risk already reported to the notifications service.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notified: Option<Risk>,
}

impl Sla {
    /// Whether the clock is running (the issue is open).
    #[must_use]
    pub const fn is_running(&self) -> bool {
        self.stopped_at.is_none()
    }

    /// The risk at `now`, or at the stop time if the clock stopped.
    #[must_use]
    pub fn risk(&self, now: OffsetDateTime) -> Risk {
        let at = self.stopped_at.unwrap_or(now);
        let total = (self.breaches_at - self.started_at).whole_seconds().max(1);
        let used = (at - self.started_at).whole_seconds();
        if used >= total {
            Risk::Red
        } else if used.saturating_mul(4) >= total.saturating_mul(3) {
            Risk::Orange
        } else if used.saturating_mul(2) >= total {
            Risk::Yellow
        } else {
            Risk::Gray
        }
    }

    /// Whether the SLA was met: the clock stopped before the deadline.
    #[must_use]
    pub fn met(&self) -> Option<bool> {
        self.stopped_at.map(|t| t <= self.breaches_at)
    }
}

/// One SLA rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlaRule {
    /// The rule name, written into the issue as the SLA type.
    pub name: String,
    /// Which issues the rule applies to.
    pub when: Filter,
    /// The time allowed, in hours.
    pub within_hours: u32,
}

/// A team's SLA rules, evaluated top to bottom.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlaPolicy {
    /// The rules; the first match applies.
    #[serde(default)]
    pub rules: Vec<SlaRule>,
}

impl Default for SlaPolicy {
    fn default() -> Self {
        Self::standard()
    }
}

impl SlaPolicy {
    /// Urgent within 24 hours, high within one week.
    #[must_use]
    pub fn standard() -> Self {
        let rule = |name: &str, priority: &str, hours| SlaRule {
            name: name.to_owned(),
            when: Condition::new(Field::Priority, Op::Is, &[priority]).into(),
            within_hours: hours,
        };
        Self {
            rules: vec![
                rule("urgent-24h", "urgent", 24),
                rule("high-1w", "high", 168),
            ],
        }
    }
}

/// What evaluating the SLA rules did to an issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlaChange {
    /// Nothing changed.
    Unchanged,
    /// A clock started (or restarted under a different rule).
    Started,
    /// The issue no longer matches any rule; its clock was removed.
    Removed,
    /// The issue closed; the clock stopped. `met` says if it was in time.
    Stopped {
        /// Whether the issue closed before the deadline.
        met: bool,
    },
    /// The issue reopened; the clock resumed with its original deadline.
    Resumed,
}

/// Applies the policy to an issue whose status category is `category`.
pub fn evaluate(
    issue: &mut Issue,
    category: Category,
    policy: &SlaPolicy,
    ctx: &EvalContext<'_>,
) -> SlaChange {
    let now = ctx.now;
    if category.is_closed() {
        return match issue.sla.as_mut() {
            Some(sla) if sla.is_running() => {
                sla.stopped_at = Some(now);
                SlaChange::Stopped {
                    met: now <= sla.breaches_at,
                }
            }
            _ => SlaChange::Unchanged,
        };
    }
    let rule = policy.rules.iter().find(|r| r.when.matches(issue, ctx));
    match (rule, issue.sla.as_mut()) {
        (None, None) => SlaChange::Unchanged,
        (None, Some(_)) => {
            issue.sla = None;
            SlaChange::Removed
        }
        (Some(rule), Some(sla)) if sla.kind == rule.name => {
            if sla.is_running() {
                SlaChange::Unchanged
            } else {
                sla.stopped_at = None;
                SlaChange::Resumed
            }
        }
        (Some(rule), _) => {
            issue.sla = Some(Sla {
                kind: rule.name.clone(),
                started_at: now,
                breaches_at: now
                    + Duration::hours(i64::from(rule.within_hours)),
                stopped_at: None,
                notified: None,
            });
            SlaChange::Started
        }
    }
}

/// A risk step-up to report: orange goes to the assignee; red (a breach)
/// goes to the assignee and the team lead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlaNotice {
    /// The new risk.
    pub risk: Risk,
    /// Who to notify.
    pub recipients: Vec<UserRef>,
}

/// Checks a running clock at `now` and records the step-up, if any.
///
/// Called by the jobs service on a timer and after every update.
pub fn tick(
    issue: &mut Issue,
    lead: Option<&UserRef>,
    now: OffsetDateTime,
) -> Option<SlaNotice> {
    let assignee = issue.assignee.clone();
    let sla = issue.sla.as_mut().filter(|s| s.is_running())?;
    let risk = sla.risk(now);
    if sla.notified.is_some_and(|n| n >= risk) || risk < Risk::Orange {
        return None;
    }
    sla.notified = Some(risk);
    let mut recipients: Vec<UserRef> = assignee.into_iter().collect();
    if risk == Risk::Red
        && let Some(lead) = lead.filter(|l| !recipients.contains(l))
    {
        recipients.push(lead.clone());
    }
    Some(SlaNotice { risk, recipients })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filter::tests::Fixed;
    use crate::id::IssueId;
    use crate::issue::IssueKey;
    use crate::scale::Priority;
    use crate::workflow::StatusKey;
    use time::macros::{date, datetime};

    fn issue(p: Priority) -> Issue {
        let mut i = Issue::new(
            "01JBZ3M8K2Q4R5S6T7V8W9X0A1".parse::<IssueId>().unwrap(),
            IssueKey::parse("ENG-1").unwrap(),
            "Outage",
            StatusKey::parse("todo").unwrap(),
            datetime!(2026-10-29 00:00 UTC),
        );
        i.priority = p;
        i.assignee = Some(UserRef::parse("dev").unwrap());
        i
    }

    fn ctx(now: OffsetDateTime) -> EvalContext<'static> {
        EvalContext {
            today: date!(2026 - 10 - 29),
            now,
            me: None,
            lookup: &Fixed,
        }
    }

    #[test]
    fn urgent_issues_get_24_hours() {
        let t0 = datetime!(2026-10-29 00:00 UTC);
        let mut i = issue(Priority::Urgent);
        let p = SlaPolicy::standard();
        assert_eq!(
            evaluate(&mut i, Category::Unstarted, &p, &ctx(t0)),
            SlaChange::Started
        );
        let sla = i.sla.clone().unwrap();
        assert_eq!(sla.kind, "urgent-24h");
        assert_eq!(sla.breaches_at, datetime!(2026-10-30 00:00 UTC));
        assert_eq!(
            evaluate(&mut i, Category::Started, &p, &ctx(t0)),
            SlaChange::Unchanged
        );

        i.priority = Priority::High;
        assert_eq!(
            evaluate(&mut i, Category::Started, &p, &ctx(t0)),
            SlaChange::Started
        );
        assert_eq!(i.sla.as_ref().unwrap().kind, "high-1w");

        i.priority = Priority::Low;
        assert_eq!(
            evaluate(&mut i, Category::Started, &p, &ctx(t0)),
            SlaChange::Removed
        );
        assert!(i.sla.is_none());
    }

    #[test]
    fn closing_stops_and_reopening_resumes() {
        let t0 = datetime!(2026-10-29 00:00 UTC);
        let mut i = issue(Priority::Urgent);
        let p = SlaPolicy::standard();
        let _ = evaluate(&mut i, Category::Unstarted, &p, &ctx(t0));
        let late = datetime!(2026-10-30 06:00 UTC);
        assert_eq!(
            evaluate(&mut i, Category::Completed, &p, &ctx(late)),
            SlaChange::Stopped { met: false }
        );
        assert_eq!(i.sla.as_ref().unwrap().met(), Some(false));
        assert_eq!(
            evaluate(&mut i, Category::Completed, &p, &ctx(late)),
            SlaChange::Unchanged
        );
        assert_eq!(
            evaluate(&mut i, Category::Started, &p, &ctx(late)),
            SlaChange::Resumed
        );
    }

    #[test]
    fn risk_steps_up_and_notifies_once() {
        let t0 = datetime!(2026-10-29 00:00 UTC);
        let mut i = issue(Priority::Urgent);
        let _ = evaluate(
            &mut i,
            Category::Unstarted,
            &SlaPolicy::standard(),
            &ctx(t0),
        );
        let sla = i.sla.clone().unwrap();
        assert_eq!(sla.risk(t0), Risk::Gray);
        assert_eq!(sla.risk(datetime!(2026-10-29 12:00 UTC)), Risk::Yellow);
        assert_eq!(sla.risk(datetime!(2026-10-29 18:00 UTC)), Risk::Orange);
        assert_eq!(sla.risk(datetime!(2026-10-30 00:00 UTC)), Risk::Red);

        let lead = UserRef::parse("lead").unwrap();
        assert!(
            tick(&mut i, Some(&lead), datetime!(2026-10-29 12:00 UTC))
                .is_none()
        );
        let n =
            tick(&mut i, Some(&lead), datetime!(2026-10-29 19:00 UTC)).unwrap();
        assert_eq!((n.risk, n.recipients.len()), (Risk::Orange, 1));
        assert!(
            tick(&mut i, Some(&lead), datetime!(2026-10-29 20:00 UTC))
                .is_none()
        );
        let n =
            tick(&mut i, Some(&lead), datetime!(2026-10-30 01:00 UTC)).unwrap();
        assert_eq!((n.risk, n.recipients.len()), (Risk::Red, 2));
        assert!(
            tick(&mut i, Some(&lead), datetime!(2026-10-30 02:00 UTC))
                .is_none()
        );
    }
}
