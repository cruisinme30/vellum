// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Structured AND/OR filters over every issue field.
//!
//! One filter language serves views, triage rules and SLA rules. There is
//! no query language in the UI (D15): a filter is a tree of conditions,
//! stored as YAML:
//!
//! ```yaml
//! and:
//!   - {field: priority, op: is, values: [urgent, high]}
//!   - or:
//!       - {field: assignee, op: is, values: ["@me"]}
//!       - {field: label, op: includes_all, values: [gateway]}
//! ```
//!
//! Values may use `@me` (the viewer), `@today`, `@today-7d`, `@today+2w`
//! (dates relative to the team's today) and `@current` (the team's current
//! cycle).

use serde::{Deserialize, Serialize};
use time::{Date, Duration, OffsetDateTime};

use crate::cycle::CycleKey;
use crate::id::{IssueId, UserRef};
use crate::issue::Issue;
use crate::scale::Priority;
use crate::sla::Risk;
use crate::team::TeamKey;
use crate::workflow::{Category, StatusKey};

/// What a filter needs to know beyond the issue itself.
pub trait Lookup {
    /// The category of a team's status.
    fn category(&self, team: &TeamKey, state: &StatusKey) -> Option<Category>;
    /// Whether an issue exists and is not closed (for `blocked`).
    fn is_open(&self, issue: &IssueId) -> bool;
    /// The team's current cycle on `today`.
    fn current_cycle(&self, team: &TeamKey, today: Date) -> Option<CycleKey>;
}

/// Where and for whom a filter is evaluated.
#[derive(Clone, Copy)]
pub struct EvalContext<'a> {
    /// Today's date in the team's time zone; resolves `@today`.
    pub today: Date,
    /// The current time; used for SLA risk.
    pub now: OffsetDateTime,
    /// The viewer; resolves `@me`. `None` makes `@me` match nothing.
    pub me: Option<&'a UserRef>,
    /// Workspace lookups.
    pub lookup: &'a dyn Lookup,
}

/// The fields a condition can test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    /// The team key.
    Team,
    /// The status key.
    Status,
    /// The status category.
    Category,
    /// Priority by name (`urgent`) or number.
    Priority,
    /// Estimate points.
    Estimate,
    /// The assignee's handle.
    Assignee,
    /// The creator's handle.
    Creator,
    /// A subscriber's handle.
    Subscriber,
    /// A label reference.
    Label,
    /// The project ID.
    Project,
    /// The milestone ID.
    Milestone,
    /// The cycle key, or `@current`.
    Cycle,
    /// The parent issue ID.
    Parent,
    /// The due date.
    Due,
    /// The creation date.
    Created,
    /// The last update date.
    Updated,
    /// The date work started.
    Started,
    /// The completion date.
    Completed,
    /// The title.
    Title,
    /// The title and body.
    Text,
    /// `true` if any blocker is still open.
    Blocked,
    /// The SLA risk (`gray`, `yellow`, `orange`, `red`); empty with no SLA.
    Sla,
}

impl Field {
    const fn is_date(self) -> bool {
        matches!(
            self,
            Self::Due | Self::Created | Self::Updated | Self::Started | Self::Completed
        )
    }
}

/// How a condition compares the field with its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// The field has any of the values.
    Is,
    /// The field has none of the values.
    IsNot,
    /// The field has every value (labels, subscribers).
    IncludesAll,
    /// The field is empty.
    IsEmpty,
    /// The field is set.
    IsNotEmpty,
    /// The text contains any value, ignoring case.
    Contains,
    /// The text contains none of the values, ignoring case.
    NotContains,
    /// The date is before the value.
    Before,
    /// The date is after the value.
    After,
    /// The date is on or before the value.
    OnOrBefore,
    /// The date is on or after the value.
    OnOrAfter,
    /// The number is less than the value.
    Lt,
    /// The number is greater than the value.
    Gt,
    /// The number is at most the value.
    Lte,
    /// The number is at least the value.
    Gte,
}

/// One test of one field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    /// The field tested.
    pub field: Field,
    /// The comparison.
    pub op: Op,
    /// The values compared against; empty for `is_empty`/`is_not_empty`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
}

impl Condition {
    /// A condition with values.
    #[must_use]
    pub fn new(field: Field, op: Op, values: &[&str]) -> Self {
        Self {
            field,
            op,
            values: values.iter().map(|v| (*v).to_owned()).collect(),
        }
    }
}

/// A tree of conditions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Filter {
    /// Every child matches (an empty list matches everything).
    And {
        /// The children.
        and: Vec<Self>,
    },
    /// Any child matches (an empty list matches nothing).
    Or {
        /// The children.
        or: Vec<Self>,
    },
    /// The child does not match.
    Not {
        /// The negated filter.
        not: Box<Self>,
    },
    /// A single condition.
    Condition(Condition),
}

impl Default for Filter {
    fn default() -> Self {
        Self::all()
    }
}

impl From<Condition> for Filter {
    fn from(c: Condition) -> Self {
        Self::Condition(c)
    }
}

impl Filter {
    /// The filter that matches every issue.
    #[must_use]
    pub const fn all() -> Self {
        Self::And { and: Vec::new() }
    }

    /// Whether the issue matches.
    #[must_use]
    pub fn matches(&self, issue: &Issue, ctx: &EvalContext<'_>) -> bool {
        match self {
            Self::And { and } => and.iter().all(|f| f.matches(issue, ctx)),
            Self::Or { or } => or.iter().any(|f| f.matches(issue, ctx)),
            Self::Not { not } => !not.matches(issue, ctx),
            Self::Condition(c) => eval(c, issue, ctx),
        }
    }

    /// Checks that every condition's operator suits its field and that its
    /// values are well formed.
    ///
    /// # Errors
    ///
    /// Returns the first problem as text.
    pub fn check(&self) -> Result<(), String> {
        match self {
            Self::And { and: list } | Self::Or { or: list } => {
                list.iter().try_for_each(Self::check)
            }
            Self::Not { not } => not.check(),
            Self::Condition(c) => check_condition(c),
        }
    }
}

fn check_condition(c: &Condition) -> Result<(), String> {
    let f = c.field;
    let needs_values = !matches!(c.op, Op::IsEmpty | Op::IsNotEmpty);
    if needs_values == c.values.is_empty() {
        return Err(if needs_values {
            format!("{:?} {:?} needs values", f, c.op)
        } else {
            format!("{:?} {:?} takes no values", f, c.op)
        });
    }
    let date_op = matches!(
        c.op,
        Op::Before | Op::After | Op::OnOrBefore | Op::OnOrAfter
    );
    let num_op = matches!(c.op, Op::Lt | Op::Gt | Op::Lte | Op::Gte);
    let text_op = matches!(c.op, Op::Contains | Op::NotContains);
    if date_op && !f.is_date() {
        return Err(format!("{f:?} is not a date field"));
    }
    if num_op && f != Field::Estimate {
        return Err(format!("{f:?} is not a number field"));
    }
    if text_op && !matches!(f, Field::Title | Field::Text) {
        return Err(format!("{f:?} is not a text field"));
    }
    for v in &c.values {
        let ok = match f {
            Field::Priority => Priority::from_name(v).is_some(),
            Field::Category => Category::from_name(v).is_some(),
            Field::Estimate => v.parse::<f64>().is_ok(),
            Field::Blocked => matches!(v.as_str(), "true" | "false"),
            Field::Sla => Risk::from_name(v).is_some(),
            _ if f.is_date() => v.starts_with("@today") || Date::parse(v, DATE).is_ok(),
            _ => true,
        };
        if !ok {
            return Err(format!("{v:?} is not a valid {f:?} value"));
        }
    }
    Ok(())
}

const DATE: &[time::format_description::BorrowedFormatItem<'_>] =
    time::macros::format_description!("[year]-[month]-[day]");

fn date_text(d: Date) -> String {
    d.format(DATE).unwrap_or_default()
}

/// Resolves `@today`, `@today-7d`, `@today+2w` or a literal date.
fn resolve_date(value: &str, today: Date) -> Option<Date> {
    let Some(rest) = value.strip_prefix("@today") else {
        return Date::parse(value, DATE).ok();
    };
    if rest.is_empty() {
        return Some(today);
    }
    let (sign, rest) = rest.split_at_checked(1)?;
    let unit_at = rest.len().checked_sub(1)?;
    let (n, unit) = rest.split_at_checked(unit_at)?;
    let n: i64 = n.parse().ok()?;
    let days = match unit {
        "d" => n,
        "w" => n.checked_mul(7)?,
        _ => return None,
    };
    match sign {
        "+" => today.checked_add(Duration::days(days)),
        "-" => today.checked_sub(Duration::days(days)),
        _ => None,
    }
}

/// The field's values on the issue, as comparable text tokens.
fn extract(field: Field, issue: &Issue, ctx: &EvalContext<'_>) -> Vec<String> {
    let opt = |o: Option<String>| o.into_iter().collect::<Vec<_>>();
    let day = |t: Option<OffsetDateTime>| opt(t.map(|t| date_text(t.date())));
    match field {
        Field::Team => vec![issue.team.to_string()],
        Field::Status => vec![issue.state.to_string()],
        Field::Category => opt(ctx
            .lookup
            .category(&issue.team, &issue.state)
            .map(|c| c.as_str().to_owned())),
        Field::Priority => vec![issue.priority.name().to_owned()],
        Field::Estimate => opt(issue.estimate.map(|e| e.points().to_string())),
        Field::Assignee => opt(issue.assignee.as_ref().map(ToString::to_string)),
        Field::Creator => opt(issue.creator.as_ref().map(ToString::to_string)),
        Field::Subscriber => issue.subscribers.iter().map(ToString::to_string).collect(),
        Field::Label => issue.labels.clone(),
        Field::Project => opt(issue.project.as_ref().map(ToString::to_string)),
        Field::Milestone => opt(issue.milestone.as_ref().map(ToString::to_string)),
        Field::Cycle => opt(issue.cycle.as_ref().map(ToString::to_string)),
        Field::Parent => opt(issue.parent.as_ref().map(ToString::to_string)),
        Field::Due => opt(issue.due.map(date_text)),
        Field::Created => day(Some(issue.created_at)),
        Field::Updated => day(Some(issue.updated_at)),
        Field::Started => day(issue.started_at),
        Field::Completed => day(issue.completed_at),
        Field::Title => vec![issue.title.clone()],
        Field::Text => vec![issue.title.clone(), issue.body.clone()],
        Field::Blocked => {
            let blocked = issue.blockers().any(|b| ctx.lookup.is_open(b));
            vec![blocked.to_string()]
        }
        Field::Sla => opt(issue
            .sla
            .as_ref()
            .filter(|s| s.is_running())
            .map(|s| s.risk(ctx.now).name().to_owned())),
    }
}

/// Resolves `@me`, `@current` and priority numbers in a value list.
fn resolve(field: Field, values: &[String], issue: &Issue, ctx: &EvalContext<'_>) -> Vec<String> {
    values
        .iter()
        .filter_map(|v| match (field, v.as_str()) {
            (_, "@me") => ctx.me.map(ToString::to_string),
            (Field::Cycle, "@current") => ctx
                .lookup
                .current_cycle(&issue.team, ctx.today)
                .map(|c| c.to_string()),
            (Field::Priority, p) => Priority::from_name(p).map(|p| p.name().to_owned()),
            (Field::Team, t) => Some(t.to_ascii_uppercase()),
            _ => Some(v.clone()),
        })
        .collect()
}

fn eval(c: &Condition, issue: &Issue, ctx: &EvalContext<'_>) -> bool {
    let tokens = extract(c.field, issue, ctx);
    let values = resolve(c.field, &c.values, issue, ctx);
    let has = |v: &String| tokens.contains(v);
    let lower = |s: &String| s.to_lowercase();
    match c.op {
        Op::Is => values.iter().any(has),
        Op::IsNot => !values.iter().any(has),
        Op::IncludesAll => !values.is_empty() && values.iter().all(has),
        Op::IsEmpty => tokens.is_empty(),
        Op::IsNotEmpty => !tokens.is_empty(),
        Op::Contains | Op::NotContains => {
            let found = tokens
                .iter()
                .map(lower)
                .any(|t| values.iter().map(lower).any(|v| t.contains(&v)));
            found == (c.op == Op::Contains)
        }
        Op::Before | Op::After | Op::OnOrBefore | Op::OnOrAfter => {
            let Some(bound) = values.first().and_then(|v| resolve_date(v, ctx.today)) else {
                return false;
            };
            tokens
                .iter()
                .filter_map(|t| Date::parse(t, DATE).ok())
                .any(|d| match c.op {
                    Op::Before => d < bound,
                    Op::After => d > bound,
                    Op::OnOrBefore => d <= bound,
                    _ => d >= bound,
                })
        }
        Op::Lt | Op::Gt | Op::Lte | Op::Gte => {
            let Some(bound) = values.first().and_then(|v| v.parse::<f64>().ok()) else {
                return false;
            };
            tokens
                .iter()
                .filter_map(|t| t.parse::<f64>().ok())
                .any(|n| match c.op {
                    Op::Lt => n < bound,
                    Op::Gt => n > bound,
                    Op::Lte => n <= bound,
                    _ => n >= bound,
                })
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::issue::{IssueKey, Relation, RelationKind};
    use crate::scale::Estimate;
    use crate::workflow::Workflow;
    use time::macros::{date, datetime};

    pub struct Fixed;

    impl Lookup for Fixed {
        fn category(&self, _: &TeamKey, state: &StatusKey) -> Option<Category> {
            Workflow::standard().unwrap().category_of(state)
        }
        fn is_open(&self, issue: &IssueId) -> bool {
            issue.to_string().ends_with('2')
        }
        fn current_cycle(&self, _: &TeamKey, _: Date) -> Option<CycleKey> {
            CycleKey::parse("2026-C10").ok()
        }
    }

    fn issue() -> Issue {
        let mut i = Issue::new(
            "01JBZ3M8K2Q4R5S6T7V8W9X0A1".parse::<IssueId>().unwrap(),
            IssueKey::parse("ENG-1").unwrap(),
            "Gateway timeout on replay",
            StatusKey::parse("in-progress").unwrap(),
            datetime!(2026-10-20 09:00 UTC),
        );
        i.priority = Priority::High;
        i.estimate = Some(Estimate::Points(3));
        i.assignee = Some(UserRef::parse("dev").unwrap());
        i.labels = vec!["gateway".into(), "type/bug".into()];
        i.due = Some(date!(2026 - 11 - 01));
        i.cycle = Some(CycleKey::parse("2026-C10").unwrap());
        i.relations = vec![Relation {
            kind: RelationKind::BlockedBy,
            issue: "01JBZ3M8K2Q4R5S6T7V8W9X0A2".parse::<IssueId>().unwrap(),
        }];
        i
    }

    fn matches(f: &Filter, me: &str) -> bool {
        let me = UserRef::parse(me).unwrap();
        let ctx = EvalContext {
            today: date!(2026 - 10 - 29),
            now: datetime!(2026-10-29 12:00 UTC),
            me: Some(&me),
            lookup: &Fixed,
        };
        f.matches(&issue(), &ctx)
    }

    fn c(field: Field, op: Op, values: &[&str]) -> Filter {
        Condition::new(field, op, values).into()
    }

    #[test]
    fn conditions_cover_every_operator() {
        let yes = [
            c(Field::Priority, Op::Is, &["urgent", "2"]),
            c(Field::Category, Op::Is, &["started"]),
            c(Field::Assignee, Op::Is, &["@me"]),
            c(Field::Label, Op::IncludesAll, &["gateway", "type/bug"]),
            c(Field::Project, Op::IsEmpty, &[]),
            c(Field::Cycle, Op::Is, &["@current"]),
            c(Field::Title, Op::Contains, &["TIMEOUT"]),
            c(Field::Text, Op::NotContains, &["flaky"]),
            c(Field::Due, Op::Before, &["@today+1w"]),
            c(Field::Due, Op::After, &["@today"]),
            c(Field::Created, Op::OnOrAfter, &["@today-9d"]),
            c(Field::Created, Op::OnOrBefore, &["2026-10-20"]),
            c(Field::Estimate, Op::Gte, &["3"]),
            c(Field::Estimate, Op::Lt, &["5"]),
            c(Field::Blocked, Op::Is, &["true"]),
            c(Field::Team, Op::Is, &["eng"]),
            c(Field::Sla, Op::IsEmpty, &[]),
        ];
        for f in &yes {
            f.check().unwrap();
            assert!(matches(f, "dev"), "{f:?}");
        }
        let no = [
            c(Field::Assignee, Op::Is, &["@me"]),
            c(Field::Label, Op::IsNot, &["gateway"]),
            c(Field::Estimate, Op::Gt, &["3"]),
            c(Field::Due, Op::Before, &["@today"]),
            c(Field::Started, Op::IsNotEmpty, &[]),
        ];
        for f in &no {
            assert!(!matches(f, "priya"), "{f:?}");
        }
    }

    #[test]
    fn trees_combine_conditions() {
        let f = Filter::And {
            and: vec![
                c(Field::Priority, Op::Is, &["high"]),
                Filter::Or {
                    or: vec![
                        c(Field::Assignee, Op::Is, &["priya"]),
                        c(Field::Label, Op::Is, &["gateway"]),
                    ],
                },
                Filter::Not {
                    not: Box::new(c(Field::Status, Op::Is, &["done"])),
                },
            ],
        };
        assert!(matches(&f, "dev"));
        assert!(matches(&Filter::all(), "dev"));
        assert!(!matches(&Filter::Or { or: vec![] }, "dev"));
    }

    #[test]
    fn filters_round_trip_through_yaml() {
        let yaml = "and:\n- field: priority\n  op: is\n  values: [urgent]\n- or:\n  - field: assignee\n    op: is\n    values: ['@me']\n  - not:\n      field: project\n      op: is_empty\n";
        let f: Filter = serde_saphyr::from_str(yaml).unwrap();
        f.check().unwrap();
        let back: Filter = serde_saphyr::from_str(&serde_saphyr::to_string(&f).unwrap()).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn bad_conditions_are_refused() {
        for f in [
            c(Field::Priority, Op::Is, &["huge"]),
            c(Field::Title, Op::Before, &["2026-01-01"]),
            c(Field::Priority, Op::Lt, &["2"]),
            c(Field::Label, Op::Contains, &["x"]),
            c(Field::Label, Op::Is, &[]),
            c(Field::Label, Op::IsEmpty, &["x"]),
            c(Field::Due, Op::Before, &["soon"]),
            c(Field::Blocked, Op::Is, &["maybe"]),
        ] {
            assert!(f.check().is_err(), "{f:?}");
        }
    }

    #[test]
    fn relative_dates_resolve() {
        let today = date!(2026 - 10 - 29);
        assert_eq!(resolve_date("@today", today), Some(today));
        assert_eq!(
            resolve_date("@today-2d", today),
            Some(date!(2026 - 10 - 27))
        );
        assert_eq!(
            resolve_date("@today+1w", today),
            Some(date!(2026 - 11 - 05))
        );
        assert_eq!(resolve_date("@today*1w", today), None);
        assert_eq!(resolve_date("@today+1y", today), None);
        assert_eq!(resolve_date("@today+", today), None);
    }
}
