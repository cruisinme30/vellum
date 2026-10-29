// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The public shapes of the Work API: what clients send and receive.
//!
//! These are deliberately separate from the engine's file types. Files can
//! grow keys without changing the API, and the API can stay stable while
//! the files evolve. IDs, keys and times are strings: ULIDs, `ENG-142`,
//! RFC 3339 timestamps and ISO dates.

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use vellum_kernel::{Error, ErrorKind};
use vellum_work::comment::{Assignment, Comment, Target, Thread};
use vellum_work::cycle::Cycle;
use vellum_work::project::{Project, ProjectProgress};
use vellum_work::{Issue, Team, WorkGraph};

// ----- shared helpers -----

/// Formats a time as RFC 3339.
pub(crate) fn rfc3339(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

fn opt_time(t: Option<OffsetDateTime>) -> Option<String> {
    t.map(rfc3339)
}

/// The serde name of an engine enum value (`in_progress`, `urgent`).
pub(crate) fn name_of<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

/// Reads an engine enum from its serde name.
pub(crate) fn parse_name<T: DeserializeOwned>(field: &str, text: &str) -> Result<T, Error> {
    serde_json::from_value(serde_json::Value::String(text.to_owned()))
        .map_err(|_| invalid(&format!("{field}: unknown value {text:?}")))
}

/// An invalid-input error.
pub(crate) fn invalid(message: &str) -> Error {
    Error::new(ErrorKind::InvalidInput, message.to_owned())
}

// ----- outputs -----

/// A team and its workflow.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TeamOut {
    /// Team ID.
    pub id: String,
    /// Key prefix, such as `ENG`.
    pub key: String,
    /// Display name.
    pub name: String,
    /// IANA time zone for cycles and recurring work.
    pub timezone: String,
    /// The team lead's handle.
    pub lead: Option<String>,
    /// Member handles.
    pub members: Vec<String>,
    /// Statuses in board order.
    pub statuses: Vec<StatusOut>,
    /// The status new issues get.
    pub default_status: String,
    /// Whether intake issues go to triage first.
    pub triage: bool,
    /// Whether the team runs cycles.
    pub cycles: bool,
    /// Label references (`group/name` or `name`).
    pub labels: Vec<String>,
    /// Template names.
    pub templates: Vec<String>,
}

/// A status in a team's workflow.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StatusOut {
    /// Slug, such as `in-review`.
    pub key: String,
    /// Display name.
    pub name: String,
    /// One of triage, backlog, unstarted, started, completed, canceled,
    /// duplicate.
    pub category: String,
}

impl TeamOut {
    pub(crate) fn of(team: &Team) -> Self {
        Self {
            id: team.id.to_string(),
            key: team.key.to_string(),
            name: team.name.clone(),
            timezone: team.timezone.clone(),
            lead: team.lead.as_ref().map(ToString::to_string),
            members: team.members.iter().map(ToString::to_string).collect(),
            statuses: team
                .workflow
                .statuses
                .iter()
                .map(|s| StatusOut {
                    key: s.key.to_string(),
                    name: s.name.clone(),
                    category: name_of(&s.category),
                })
                .collect(),
            default_status: team.workflow.default.to_string(),
            triage: team.triage,
            cycles: team.cycles.is_some(),
            labels: team
                .labels
                .iter()
                .map(vellum_work::label::Label::reference)
                .collect(),
            templates: team.templates.iter().map(|t| t.name.clone()).collect(),
        }
    }
}

/// An issue.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct IssueOut {
    /// Issue ID; stable across moves.
    pub id: String,
    /// Identifier, such as `ENG-142`.
    pub key: String,
    /// Team key.
    pub team: String,
    /// Title.
    pub title: String,
    /// Status slug.
    pub state: String,
    /// The status's category.
    pub category: String,
    /// Priority 0 (none) to 4 (low); 1 is urgent.
    pub priority: u8,
    /// Estimate as written (`3`, `M`).
    pub estimate: Option<String>,
    /// Assignee handle.
    pub assignee: Option<String>,
    /// Creator handle.
    pub creator: Option<String>,
    /// Label references.
    pub labels: Vec<String>,
    /// Project ID.
    pub project: Option<String>,
    /// Milestone ID.
    pub milestone: Option<String>,
    /// Cycle key, such as `2026-C44`.
    pub cycle: Option<String>,
    /// Parent issue key.
    pub parent: Option<String>,
    /// Relations to other issues.
    pub relations: Vec<RelationOut>,
    /// Subscriber handles.
    pub subscribers: Vec<String>,
    /// Earlier identifiers that still resolve.
    pub aliases: Vec<String>,
    /// Links: pull requests, the source block, documents.
    pub links: Vec<LinkOut>,
    /// Due date.
    pub due: Option<String>,
    /// The running or last SLA.
    pub sla: Option<SlaOut>,
    /// Created.
    pub created_at: String,
    /// Last updated.
    pub updated_at: String,
    /// Work started.
    pub started_at: Option<String>,
    /// Completed.
    pub completed_at: Option<String>,
    /// Canceled.
    pub canceled_at: Option<String>,
    /// The description, in Vellum Markdown; left out of lists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A relation from one issue to another.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RelationOut {
    /// `blocks`, `blocked_by`, `related` or `duplicate_of`.
    pub kind: String,
    /// The other issue's key.
    pub issue: String,
}

/// A link on an issue.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct LinkOut {
    /// URL; `vellum://doc/<id>#blk-…` for document blocks.
    pub url: String,
    /// Title.
    pub title: Option<String>,
    /// `source`, `doc`, `pull_request` or another kind.
    pub kind: Option<String>,
}

/// An SLA clock.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SlaOut {
    /// The SLA rule's name.
    pub kind: String,
    /// When it breaches.
    pub breaches_at: String,
    /// When it stopped (closed or met).
    pub stopped_at: Option<String>,
    /// gray, yellow, orange or red.
    pub risk: String,
}

impl IssueOut {
    pub(crate) fn of(issue: &Issue, graph: &WorkGraph, now: OffsetDateTime, full: bool) -> Self {
        let key_of = |id: &vellum_work::IssueId| {
            graph
                .issue(id)
                .map_or_else(|| id.to_string(), |i| i.key().to_string())
        };
        Self {
            id: issue.id.to_string(),
            key: issue.key().to_string(),
            team: issue.team.to_string(),
            title: issue.title.clone(),
            state: issue.state.to_string(),
            category: graph
                .category_of(issue)
                .map(|c| name_of(&c))
                .unwrap_or_default(),
            priority: issue.priority.number(),
            estimate: issue.estimate.map(|e| e.to_string()),
            assignee: issue.assignee.as_ref().map(ToString::to_string),
            creator: issue.creator.as_ref().map(ToString::to_string),
            labels: issue.labels.clone(),
            project: issue.project.map(|p| p.to_string()),
            milestone: issue.milestone.map(|m| m.to_string()),
            cycle: issue.cycle.as_ref().map(ToString::to_string),
            parent: issue.parent.as_ref().map(key_of),
            relations: issue
                .relations
                .iter()
                .map(|r| RelationOut {
                    kind: name_of(&r.kind),
                    issue: key_of(&r.issue),
                })
                .collect(),
            subscribers: issue.subscribers.iter().map(ToString::to_string).collect(),
            aliases: issue.aliases.iter().map(ToString::to_string).collect(),
            links: issue
                .links
                .iter()
                .map(|l| LinkOut {
                    url: l.url.clone(),
                    title: l.title.clone(),
                    kind: l.kind.clone(),
                })
                .collect(),
            due: issue.due.map(|d| d.to_string()),
            sla: issue.sla.as_ref().map(|s| SlaOut {
                kind: s.kind.clone(),
                breaches_at: rfc3339(s.breaches_at),
                stopped_at: opt_time(s.stopped_at),
                risk: name_of(&s.risk(now)),
            }),
            created_at: rfc3339(issue.created_at),
            updated_at: rfc3339(issue.updated_at),
            started_at: opt_time(issue.started_at),
            completed_at: opt_time(issue.completed_at),
            canceled_at: opt_time(issue.canceled_at),
            description: full.then(|| issue.body.clone()),
        }
    }
}

/// The result of creating an issue.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CreatedIssue {
    /// The new issue.
    pub issue: IssueOut,
    /// For an issue filed from a block: the chip the caller writes into
    /// the document (the document belongs to the drive, not Work).
    pub chip: Option<ChipOut>,
}

/// Where to write an issue chip.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ChipOut {
    /// Document ID.
    pub doc: String,
    /// Block ID.
    pub block: String,
    /// The key to write.
    pub key: String,
}

/// A list of issues in groups.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct IssueGroups {
    /// Groups in display order.
    pub groups: Vec<GroupOut>,
}

/// One group of issues.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GroupOut {
    /// The group's value (`in-review`, `urgent`, `dev`); `None` is "none".
    pub key: Option<String>,
    /// The issues, in order.
    pub issues: Vec<IssueOut>,
}

/// An issue in My Issues, with why it sits where it does.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FocusedIssue {
    /// urgent, sla, blocked, cycle, active, triage, backlog or done.
    pub focus: String,
    /// The issue.
    pub issue: IssueOut,
}

/// My Issues.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct MyIssuesOut {
    /// Issues in focus order.
    pub issues: Vec<FocusedIssue>,
}

/// A team's cycles.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CyclesOut {
    /// Cycles by start date.
    pub cycles: Vec<CycleOut>,
    /// Mean completed points over the last three cycles.
    pub velocity: Option<f64>,
}

/// A cycle.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CycleOut {
    /// Key, such as `2026-C44`.
    pub key: String,
    /// Display name.
    pub name: Option<String>,
    /// First day.
    pub starts_on: String,
    /// Day after the last.
    pub ends_on: String,
    /// Completed points recorded so far.
    pub completed_points: u32,
    /// Daily burn-up: date, scope points, completed points.
    pub burn_up: Vec<BurnOut>,
}

/// One burn-up point.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct BurnOut {
    /// The day.
    pub date: String,
    /// Points in scope.
    pub scope: u32,
    /// Points completed.
    pub completed: u32,
}

impl CycleOut {
    pub(crate) fn of(c: &Cycle) -> Self {
        Self {
            key: c.key.to_string(),
            name: c.name.clone(),
            starts_on: c.starts_on.to_string(),
            ends_on: c.ends_on.to_string(),
            completed_points: c.completed_points(),
            burn_up: c
                .burn_up
                .iter()
                .map(|p| BurnOut {
                    date: p.date.to_string(),
                    scope: p.scope,
                    completed: p.completed,
                })
                .collect(),
        }
    }
}

/// A project with its computed progress.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectOut {
    /// Project ID.
    pub id: String,
    /// Title.
    pub title: String,
    /// planned, started, paused, completed or canceled.
    pub status: String,
    /// Lead handle.
    pub lead: Option<String>,
    /// Team keys.
    pub teams: Vec<String>,
    /// Target date.
    pub target_date: Option<String>,
    /// Issues in the project.
    pub issues: u32,
    /// Percent complete by points.
    pub percent: u8,
    /// The project file, front matter and spec.
    pub markdown: String,
}

impl ProjectOut {
    pub(crate) fn of(p: &Project, progress: &ProjectProgress) -> Self {
        Self {
            id: p.id.to_string(),
            title: p.title.clone(),
            status: name_of(&p.status),
            lead: p.lead.as_ref().map(ToString::to_string),
            teams: p.teams.iter().map(ToString::to_string).collect(),
            target_date: p.target_date.map(|d| d.to_string()),
            issues: progress.overall.issues,
            percent: progress.overall.percent(),
            markdown: p.to_markdown().unwrap_or_default(),
        }
    }
}

/// A comment thread.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ThreadOut {
    /// Thread ID (the first comment's ID).
    pub id: String,
    /// What the thread is on.
    pub target: TargetOut,
    /// The quoted text.
    pub quote: Option<String>,
    /// Assignee handle ("assign to").
    pub assignee: Option<String>,
    /// When resolved; unset while open.
    pub resolved_at: Option<String>,
    /// Who resolved it.
    pub resolved_by: Option<String>,
    /// Comments, oldest first.
    pub comments: Vec<CommentOut>,
}

/// What a thread is on.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TargetOut {
    /// The issue key, for an issue.
    pub issue: Option<String>,
    /// The document ID, for a page or block.
    pub doc: Option<String>,
    /// The block ID, for a block.
    pub block: Option<String>,
}

/// A comment.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CommentOut {
    /// Comment ID.
    pub id: String,
    /// Author handle.
    pub author: String,
    /// Whether a model wrote it for the author.
    pub by_model: bool,
    /// Posted.
    pub created_at: String,
    /// Last edited.
    pub edited_at: Option<String>,
    /// Body in Vellum Markdown; empty once deleted.
    pub body: String,
    /// Reactions: emoji and the handles that chose it.
    pub reactions: Vec<ReactionOut>,
    /// Deleted, kept as a marker for its replies.
    pub deleted: bool,
}

/// People who reacted with one emoji.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ReactionOut {
    /// The emoji.
    pub emoji: String,
    /// Handles.
    pub by: Vec<String>,
}

impl TargetOut {
    pub(crate) fn of(t: &Target, graph: &WorkGraph) -> Self {
        match t {
            Target::Issue { issue } => Self {
                issue: Some(
                    graph
                        .issue(issue)
                        .map_or_else(|| issue.to_string(), |i| i.key().to_string()),
                ),
                doc: None,
                block: None,
            },
            Target::Doc { doc, block } => Self {
                issue: None,
                doc: Some(doc.to_string()),
                block: block.clone(),
            },
        }
    }
}

impl CommentOut {
    fn of(c: &Comment) -> Self {
        Self {
            id: c.id.to_string(),
            author: c.author.to_string(),
            by_model: c.authored_by == vellum_work::comment::AuthoredBy::Model,
            created_at: rfc3339(c.created_at),
            edited_at: opt_time(c.edited_at),
            body: c.body.clone(),
            reactions: c
                .reactions
                .iter()
                .map(|(emoji, by)| ReactionOut {
                    emoji: emoji.clone(),
                    by: by.iter().map(ToString::to_string).collect(),
                })
                .collect(),
            deleted: c.deleted,
        }
    }
}

impl ThreadOut {
    pub(crate) fn of(t: &Thread, graph: &WorkGraph) -> Self {
        Self {
            id: t.id.to_string(),
            target: TargetOut::of(&t.target, graph),
            quote: t.quote.clone(),
            assignee: t.assignee.as_ref().map(ToString::to_string),
            resolved_at: opt_time(t.resolved_at),
            resolved_by: t.resolved_by.as_ref().map(ToString::to_string),
            comments: t.comments.iter().map(CommentOut::of).collect(),
        }
    }
}

/// Threads on one document or issue.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ThreadsOut {
    /// Open threads first, then resolved, each oldest first.
    pub threads: Vec<ThreadOut>,
}

/// The result of a comment command.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CommentResult {
    /// The thread after the change; `None` when it was removed.
    pub thread: Option<ThreadOut>,
}

/// Open "assign to" items.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AssignmentsOut {
    /// Oldest first.
    pub assignments: Vec<AssignmentOut>,
}

/// One "assign to" item.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AssignmentOut {
    /// Thread ID.
    pub thread: String,
    /// Where it is.
    pub target: TargetOut,
    /// Who assigned it.
    pub assigned_by: Option<String>,
    /// When.
    pub assigned_at: Option<String>,
    /// The quoted text or the start of the comment.
    pub summary: String,
}

impl AssignmentOut {
    pub(crate) fn of(a: &Assignment, graph: &WorkGraph) -> Self {
        Self {
            thread: a.thread.to_string(),
            target: TargetOut::of(&a.target, graph),
            assigned_by: a.assigned_by.as_ref().map(ToString::to_string),
            assigned_at: opt_time(a.assigned_at),
            summary: a.summary.clone(),
        }
    }
}

/// Nothing to return.
#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
pub struct Done {
    /// Always true.
    pub ok: bool,
}

// ----- inputs -----

/// No input.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct NoInput {}

/// A team by key.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TeamRef {
    /// Team key, such as `ENG`.
    pub team: String,
}

/// An issue by identifier.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct IssueRef {
    /// `ENG-142`, an old alias, or the issue ID.
    pub key: String,
}

/// A new issue.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateIssue {
    /// Team key.
    pub team: String,
    /// Title.
    pub title: String,
    /// Description in Vellum Markdown.
    #[serde(default)]
    pub description: String,
    /// Template name, applied first.
    pub template: Option<String>,
    /// Status slug; the team default (or triage, for intake) when unset.
    pub state: Option<String>,
    /// Priority 0–4.
    pub priority: Option<u8>,
    /// Estimate (`3`, `M`).
    pub estimate: Option<String>,
    /// Assignee handle.
    pub assignee: Option<String>,
    /// Label references.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Project ID.
    pub project: Option<String>,
    /// Parent issue key.
    pub parent: Option<String>,
    /// Due date, `YYYY-MM-DD`.
    pub due: Option<String>,
    /// `<doc id>#blk-…` when filed from a page.
    pub source_block: Option<String>,
    /// The selected text, for a block.
    pub quote: Option<String>,
    /// Intake (form, email, Slack): goes to triage when the team uses it.
    #[serde(default)]
    pub intake: bool,
}

/// Changes to an issue. Absent fields are kept; an empty string clears.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UpdateIssue {
    /// The issue (`ENG-142`, alias or ID).
    pub key: String,
    /// New title.
    pub title: Option<String>,
    /// New description.
    pub description: Option<String>,
    /// New status slug.
    pub state: Option<String>,
    /// New priority 0–4.
    pub priority: Option<u8>,
    /// New estimate; empty clears.
    pub estimate: Option<String>,
    /// New assignee handle; empty clears.
    pub assignee: Option<String>,
    /// Labels to add.
    #[serde(default)]
    pub add_labels: Vec<String>,
    /// Labels to remove.
    #[serde(default)]
    pub remove_labels: Vec<String>,
    /// Project ID; empty clears.
    pub project: Option<String>,
    /// Cycle key; empty clears.
    pub cycle: Option<String>,
    /// Parent issue key; empty clears.
    pub parent: Option<String>,
    /// Due date; empty clears.
    pub due: Option<String>,
    /// Relations to add.
    #[serde(default)]
    pub add_relations: Vec<RelationIn>,
    /// Relations to remove.
    #[serde(default)]
    pub remove_relations: Vec<RelationIn>,
    /// Subscribe (`true`) or unsubscribe the caller.
    pub subscribe: Option<bool>,
}

/// A relation to add or remove.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RelationIn {
    /// `blocks`, `blocked_by`, `related` or `duplicate_of`.
    pub kind: String,
    /// The other issue's key.
    pub issue: String,
}

/// Move an issue to another team.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MoveIssue {
    /// The issue.
    pub key: String,
    /// The new team's key.
    pub to: String,
}

/// A triage decision.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TriageIssue {
    /// The issue.
    pub key: String,
    /// `accept`, `decline`, `duplicate` or `snooze`.
    pub action: String,
    /// For accept: the status to move to.
    pub state: Option<String>,
    /// For decline: why.
    pub reason: Option<String>,
    /// For duplicate: the original issue's key.
    pub duplicate_of: Option<String>,
    /// For snooze: until when (RFC 3339).
    pub until: Option<String>,
}

/// Which issues to list, and how.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct QueryIssues {
    /// Limit to one team.
    pub team: Option<String>,
    /// A Work filter as JSON text, such as
    /// `{"field":"assignee","op":"is","values":["@me"]}`.
    pub filter: Option<String>,
    /// status, assignee, priority, project, cycle, label, team or none.
    pub group_by: Option<String>,
    /// priority, updated, created, due, estimate or title.
    pub order_by: Option<String>,
    /// list or board.
    pub layout: Option<String>,
    /// `all`, `past_week`, `past_month` or `none`.
    pub show_closed: Option<String>,
    /// At most this many issues (default 250, at most 1000).
    pub limit: Option<usize>,
}

/// My Issues.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct MyIssues {
    /// assigned (default), created or subscribed.
    pub tab: Option<String>,
}

/// Replace a team's configuration.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PutTeam {
    /// Team key; must match the file.
    pub team: String,
    /// The whole `team.yaml`.
    pub yaml: String,
}

/// Create a team.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateTeam {
    /// Key prefix, two to seven characters, such as `ENG`.
    pub key: String,
    /// Display name.
    pub name: String,
    /// Lead handle.
    pub lead: Option<String>,
    /// Member handles.
    #[serde(default)]
    pub members: Vec<String>,
}

/// A project by ID.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ProjectRef {
    /// Project ID.
    pub project_id: String,
}

/// Create or replace a project.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PutProject {
    /// Project ID; must match the file.
    pub project_id: String,
    /// The whole `project.md`.
    pub markdown: String,
}

/// Threads on a target.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ListComments {
    /// An issue key, a document ID, or `<doc id>#blk-…`.
    pub target: String,
}

/// A new comment or reply.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AddComment {
    /// Where to start a thread: an issue key, a document ID or
    /// `<doc id>#blk-…`. Leave unset for a reply.
    pub target: Option<String>,
    /// The comment to reply to.
    pub reply_to: Option<String>,
    /// The selected text, for a block thread.
    pub quote: Option<String>,
    /// Body in Vellum Markdown.
    pub body: String,
    /// "Assign to" this handle.
    pub assign_to: Option<String>,
    /// Written by a model for the user (MCP); shown with a badge.
    #[serde(default)]
    pub by_model: bool,
}

/// Edit a comment.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EditComment {
    /// Comment ID.
    pub comment_id: String,
    /// New body.
    pub body: String,
}

/// A comment by ID.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CommentRef {
    /// Comment ID.
    pub comment_id: String,
}

/// Add or remove a reaction.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct React {
    /// Comment ID.
    pub comment_id: String,
    /// The emoji.
    pub emoji: String,
    /// Add (`true`, the default) or remove.
    #[serde(default = "yes")]
    pub on: bool,
}

const fn yes() -> bool {
    true
}

/// Resolve or reopen a thread.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ResolveThread {
    /// Thread ID.
    pub thread_id: String,
    /// Resolve (`true`, the default) or reopen.
    #[serde(default = "yes")]
    pub resolved: bool,
}

/// Assign a thread.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AssignThread {
    /// Thread ID.
    pub thread_id: String,
    /// Handle; unset or empty unassigns.
    pub assignee: Option<String>,
}
