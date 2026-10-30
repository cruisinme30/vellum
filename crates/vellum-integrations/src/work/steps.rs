// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The [`WorkCommands`](crate::model::WorkCommands) calls as plain,
//! synchronous steps on the issue tracker engine.
//!
//! Each step takes the engine's [`WorkGraph`] (and [`Comments`]) by
//! reference and returns what it changed, so any host can run it under its
//! own lock and commit the result its own way. [`GraphWork`](super::GraphWork)
//! runs them over a shared graph; `vellum serve` runs them inside the Work
//! service's `transact`, so integration changes are written and announced
//! exactly like changes made in the app.

use jiff::Timestamp;
use vellum_work::issue::IssueLink;
use vellum_work::{
    Category, Changes, CommentChanges, Comments, Ctx, Estimate, IssueId,
    NewComment, NewIssue, StatusKey, Target, TeamKey, UserRef, WorkGraph,
};

use crate::model::{
    IssueDraft, IssueKey, IssueSnapshot, Priority, Source, StateCategory,
    WorkError, WorkflowState,
};

/// The issue `key` names, following aliases of moved issues.
///
/// # Errors
///
/// The issue's team or state is missing from the graph.
pub fn snapshot(
    graph: &WorkGraph,
    key: &IssueKey,
) -> Result<Option<IssueSnapshot>, WorkError> {
    let Some(issue) = graph.resolve(&key.to_string()) else {
        return Ok(None);
    };
    let team = graph
        .team(&issue.team)
        .ok_or_else(|| WorkError::NotFound(issue.team.to_string()))?;
    let status = team
        .workflow
        .get(&issue.state)
        .ok_or_else(|| WorkError::NotFound(issue.state.to_string()))?;
    Ok(Some(IssueSnapshot {
        key: our_key(&issue.team, issue.number),
        title: issue.title.clone(),
        state: state(status),
        assignee: issue.assignee.as_ref().map(ToString::to_string),
        created_at: from_offset(issue.created_at)?,
    }))
}

/// The team's workflow states, in board order.
///
/// # Errors
///
/// [`WorkError::NotFound`] for an unknown team.
pub fn states(
    graph: &WorkGraph,
    team: &str,
) -> Result<Vec<WorkflowState>, WorkError> {
    let key = team_key(team)?;
    let team = graph
        .team(&key)
        .ok_or_else(|| WorkError::NotFound(key.to_string()))?;
    Ok(team.workflow.statuses.iter().map(state).collect())
}

/// The Git mapping the team set in its settings, if any (R-WK-10).
///
/// # Errors
///
/// [`WorkError::NotFound`] for an unknown team.
pub fn git_mapping(
    graph: &WorkGraph,
    team: &str,
) -> Result<Option<crate::git::Mapping>, WorkError> {
    let key = team_key(team)?;
    let team = graph
        .team(&key)
        .ok_or_else(|| WorkError::NotFound(key.to_string()))?;
    let name = |s: &Option<StatusKey>| s.as_ref().map(ToString::to_string);
    Ok(team.git.as_ref().map(|g| crate::git::Mapping {
        branch_pushed: name(&g.branch_pushed),
        pr_opened: name(&g.pr_opened),
        pr_approved: name(&g.pr_approved),
        pr_merged: name(&g.pr_merged),
        pr_closed_unmerged: name(&g.pr_closed_unmerged),
    }))
}

/// The team an issue belongs to, for picking its time zone.
///
/// # Errors
///
/// [`WorkError::NotFound`] for an unknown issue.
pub fn team_of(
    graph: &WorkGraph,
    key: &IssueKey,
) -> Result<TeamKey, WorkError> {
    find(graph, key).map(|(_, team)| team)
}

/// Moves an issue to the state `state_id`.
///
/// # Errors
///
/// An unknown issue or state, or a move the engine refuses.
pub fn set_state(
    graph: &mut WorkGraph,
    key: &IssueKey,
    state_id: &str,
    ctx: &Ctx,
) -> Result<Changes, WorkError> {
    let status = StatusKey::parse(state_id).map_err(refused)?;
    let (id, _) = find(graph, key)?;
    graph.set_state(&id, status, ctx).map_err(engine)
}

/// Files an issue from `draft`, or returns the issue already filed from
/// the same source (then with no changes).
///
/// # Errors
///
/// An unknown team, parent or user, or an issue the engine refuses.
pub fn create_issue(
    graph: &mut WorkGraph,
    draft: IssueDraft,
    ctx: &Ctx,
) -> Result<(IssueKey, Option<Changes>), WorkError> {
    let team = team_key(&draft.team)?;
    let source = source_link(&draft.source);
    if let Some(link) = &source
        && let Some(existing) = graph
            .issues()
            .find(|i| i.links.iter().any(|l| l.url == link.url))
    {
        let key = our_key(&existing.team, existing.number);
        tracing::info!(
            issue = %key,
            source = %link.url,
            "intake already filed"
        );
        return Ok((key, None));
    }
    let id = IssueId::generate();
    let new = new_issue(graph, id, team, draft, source)?;
    let changes = graph.create_issue(new, ctx).map_err(engine)?;
    let issue = graph.issue(&id).ok_or_else(|| {
        WorkError::Unavailable("created issue vanished".to_owned())
    })?;
    Ok((our_key(&issue.team, issue.number), Some(changes)))
}

/// Adds a comment to an issue as `ctx`'s actor.
///
/// # Errors
///
/// An unknown issue, or a comment the engine refuses.
pub fn add_comment(
    graph: &mut WorkGraph,
    comments: &mut Comments,
    key: &IssueKey,
    body: &str,
    ctx: &Ctx,
) -> Result<(CommentChanges, Changes), WorkError> {
    let (id, _) = find(graph, key)?;
    let audience = graph
        .issue(&id)
        .map(|i| i.subscribers.clone())
        .unwrap_or_default();
    let draft = NewComment::new(
        vellum_kernel::CommentId::generate(),
        Target::Issue { issue: id },
        body,
    );
    let added = comments.add(draft, &audience, ctx).map_err(engine)?;
    let changes = graph.apply_comment(&added, ctx).map_err(engine)?;
    Ok((added, changes))
}

/// The user a comment is written as: its author when known, else `bot`.
///
/// # Errors
///
/// An author that is not a valid handle.
pub fn actor(
    author: Option<&str>,
    bot: &UserRef,
) -> Result<UserRef, WorkError> {
    author
        .map_or_else(|| Ok(bot.clone()), |h| UserRef::parse(h).map_err(refused))
}

/// Builds the engine's draft from ours.
fn new_issue(
    graph: &WorkGraph,
    id: IssueId,
    team: TeamKey,
    draft: IssueDraft,
    source: Option<IssueLink>,
) -> Result<NewIssue, WorkError> {
    let intake = matches!(
        draft.source,
        Source::Slack { .. }
            | Source::Sentry { .. }
            | Source::Email { .. }
            | Source::Form { .. }
    );
    let mut new = NewIssue::new(id, team, &draft.title);
    new.body = draft.description;
    new.intake = intake;
    new.priority = draft.priority.map(priority);
    // Outside sources name labels the team may not have (Sentry adds
    // `sentry`); filing the issue matters more, so unknown ones are dropped.
    let known = |label: &str| {
        graph
            .team(&new.team)
            .into_iter()
            .flat_map(|t| t.labels.iter())
            .chain(graph.workspace_labels())
            .any(|l| l.reference().eq_ignore_ascii_case(label))
    };
    let labels: Vec<String> =
        draft.labels.into_iter().filter(|l| known(l)).collect();
    new.labels = labels;
    new.assignee = draft
        .assignee
        .as_deref()
        .map(UserRef::parse)
        .transpose()
        .map_err(refused)?;
    new.estimate = draft.estimate.as_deref().and_then(Estimate::from_text);
    new.project = draft.project.as_deref().and_then(|p| {
        graph
            .projects()
            .find(|project| {
                project.id.to_string() == p
                    || project.title.eq_ignore_ascii_case(p)
            })
            .map(|project| project.id)
    });
    new.due = draft.due.map(to_date).transpose()?;
    new.state = draft
        .state
        .as_deref()
        .map(StatusKey::parse)
        .transpose()
        .map_err(refused)?;
    new.parent = draft
        .parent
        .map(|p| find(graph, &p).map(|(parent, _)| parent))
        .transpose()?;
    new.links.extend(source);
    Ok(new)
}

/// The link that identifies where an issue came from, used to avoid
/// filing the same report twice.
fn source_link(source: &Source) -> Option<IssueLink> {
    let (url, kind, title) = match source {
        Source::Slack {
            permalink: Some(url),
        } => (url.clone(), "slack", "Slack message"),
        Source::Sentry { issue_id } => {
            (format!("sentry:issue/{issue_id}"), "sentry", "Sentry issue")
        }
        Source::Email { message_id, .. } => (
            format!("mid:{}", message_id.trim_matches(['<', '>'])),
            "email",
            "Email",
        ),
        Source::Import {
            tracker,
            external_id,
        } => (
            format!("import:{tracker}/{external_id}"),
            "import",
            "Imported issue",
        ),
        Source::App
        | Source::Slack { permalink: None }
        | Source::Form { .. }
        | Source::Recurring { .. } => {
            return None;
        }
    };
    Some(IssueLink {
        url,
        title: Some(title.to_owned()),
        kind: Some(kind.to_owned()),
    })
}

fn find(
    graph: &WorkGraph,
    key: &IssueKey,
) -> Result<(IssueId, TeamKey), WorkError> {
    graph
        .resolve(&key.to_string())
        .map(|i| (i.id, i.team.clone()))
        .ok_or_else(|| WorkError::NotFound(key.to_string()))
}

pub(super) fn team_key(team: &str) -> Result<TeamKey, WorkError> {
    TeamKey::parse(team).map_err(|_| WorkError::NotFound(team.to_owned()))
}

fn our_key(team: &TeamKey, number: u64) -> IssueKey {
    IssueKey {
        team: team.to_string(),
        number,
    }
}

fn state(status: &vellum_work::Status) -> WorkflowState {
    WorkflowState {
        id: status.key.to_string(),
        name: status.name.clone(),
        category: match status.category {
            Category::Triage => StateCategory::Triage,
            Category::Backlog => StateCategory::Backlog,
            Category::Unstarted => StateCategory::Unstarted,
            Category::Started => StateCategory::Started,
            Category::Completed => StateCategory::Completed,
            Category::Canceled => StateCategory::Canceled,
            Category::Duplicate => StateCategory::Duplicate,
        },
    }
}

const fn priority(p: Priority) -> vellum_work::Priority {
    match p {
        Priority::None => vellum_work::Priority::None,
        Priority::Urgent => vellum_work::Priority::Urgent,
        Priority::High => vellum_work::Priority::High,
        Priority::Normal => vellum_work::Priority::Normal,
        Priority::Low => vellum_work::Priority::Low,
    }
}

fn engine(error: vellum_work::WorkError) -> WorkError {
    match error {
        vellum_work::WorkError::UnknownIssue(what) => WorkError::NotFound(what),
        vellum_work::WorkError::UnknownTeam(team) => {
            WorkError::NotFound(team.to_string())
        }
        other => WorkError::Refused(other.to_string()),
    }
}

fn refused(error: impl std::fmt::Display) -> WorkError {
    WorkError::Refused(error.to_string())
}

pub(super) fn to_offset(
    ts: Timestamp,
) -> Result<time::OffsetDateTime, WorkError> {
    time::OffsetDateTime::from_unix_timestamp_nanos(ts.as_nanosecond())
        .map_err(|e| WorkError::Refused(format!("timestamp out of range: {e}")))
}

fn from_offset(at: time::OffsetDateTime) -> Result<Timestamp, WorkError> {
    Timestamp::from_nanosecond(at.unix_timestamp_nanos())
        .map_err(|e| WorkError::Refused(format!("timestamp out of range: {e}")))
}

pub(super) fn to_date(
    date: jiff::civil::Date,
) -> Result<time::Date, WorkError> {
    let bad = |e: time::error::ComponentRange| {
        WorkError::Refused(format!("bad date: {e}"))
    };
    let month = u8::try_from(date.month())
        .map_err(|_| WorkError::Refused("bad month".to_owned()))
        .and_then(|m| time::Month::try_from(m).map_err(bad))?;
    let day = u8::try_from(date.day())
        .map_err(|_| WorkError::Refused("bad day".to_owned()))?;
    time::Date::from_calendar_date(i32::from(date.year()), month, day)
        .map_err(bad)
}

/// The issues an earlier import from `tracker` created, by source ID.
#[must_use]
pub fn imported(
    graph: &WorkGraph,
    tracker: &str,
) -> std::collections::BTreeMap<String, IssueKey> {
    let prefix = format!("import:{tracker}/");
    graph
        .issues()
        .flat_map(|issue| {
            issue.links.iter().filter_map(|link| {
                link.url.strip_prefix(&prefix).map(|external| {
                    (external.to_owned(), our_key(&issue.team, issue.number))
                })
            })
        })
        .collect()
}

/// Creates every issue of `plan`, oldest first, as `ctx`'s actor, and
/// returns each one's key by source ID with what changed. An issue whose
/// source was imported before is not created again.
///
/// # Errors
///
/// The first issue the engine refuses; nothing is kept then.
pub fn import_plan(
    graph: &mut WorkGraph,
    plan: &crate::import::Plan,
    ctx: &Ctx,
) -> Result<(std::collections::BTreeMap<String, IssueKey>, Changes), WorkError>
{
    let mut keys = std::collections::BTreeMap::new();
    let mut all = Changes::default();
    for planned in &plan.create {
        let (key, changes) = create_issue(graph, planned.draft.clone(), ctx)?;
        if let Some(changes) = changes {
            all.merge(changes);
        }
        let _ = keys.insert(planned.external_id.clone(), key);
    }
    Ok((keys, all))
}

/// Sets the parents and relations an import found, once every issue
/// exists.
///
/// # Errors
///
/// An issue that vanished, or a link the engine refuses (a parent cycle).
pub fn apply_links(
    graph: &mut WorkGraph,
    links: &[crate::import::Link],
    ctx: &Ctx,
) -> Result<Changes, WorkError> {
    use crate::import::{Link, Relation};
    use vellum_work::issue::{Relation as EngineRelation, RelationKind};
    let mut all = Changes::default();
    for link in links {
        let (from, patch) = match link {
            Link::Parent { child, parent } => {
                let (parent, _) = find(graph, parent)?;
                (
                    child,
                    vellum_work::IssuePatch {
                        parent: Some(Some(parent)),
                        ..vellum_work::IssuePatch::default()
                    },
                )
            }
            Link::Relation { from, relation, to } => {
                let (to, _) = find(graph, to)?;
                let kind = match relation {
                    Relation::Blocks => RelationKind::Blocks,
                    Relation::BlockedBy => RelationKind::BlockedBy,
                    Relation::Related => RelationKind::Related,
                    Relation::DuplicateOf => RelationKind::DuplicateOf,
                };
                (
                    from,
                    vellum_work::IssuePatch {
                        add_relations: vec![EngineRelation { kind, issue: to }],
                        ..vellum_work::IssuePatch::default()
                    },
                )
            }
        };
        let (id, _) = find(graph, from)?;
        all.merge(graph.update_issue(&id, patch, ctx).map_err(engine)?);
    }
    Ok(all)
}
