// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Work operations: one entry per REST route and GraphQL field.
//!
//! Reads need `read`, issue and comment writes need `work`, and team
//! configuration needs `admin` and a recent sign-in (D136).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use vellum_contracts::{HttpMethod, Operation, RequestContext, Scope};
use vellum_kernel::Error;

use crate::api::{ProjectOut, TeamOut};
use crate::service::WorkService;

/// Every team.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TeamsOut {
    /// Teams by key.
    pub teams: Vec<TeamOut>,
}

/// Every project.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectsOut {
    /// Projects.
    pub projects: Vec<ProjectOut>,
}

/// How recent a sign-in team configuration needs.
const FRESH: Duration = Duration::from_mins(10);

fn query<I, O, F, Fut>(
    svc: &Arc<WorkService>,
    id: &'static str,
    path: &'static str,
    f: F,
) -> Operation
where
    F: Fn(Arc<WorkService>, RequestContext, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, Error>> + Send + 'static,
    I: DeserializeOwned + JsonSchema + Send + 'static,
    O: Serialize + JsonSchema + Send + 'static,
{
    let svc = Arc::clone(svc);
    Operation::query(id, path, Scope::Read, move |ctx, input| {
        f(Arc::clone(&svc), ctx, input)
    })
}

fn mutation<I, O, F, Fut>(
    svc: &Arc<WorkService>,
    id: &'static str,
    method: HttpMethod,
    path: &'static str,
    scope: Scope,
    f: F,
) -> Operation
where
    F: Fn(Arc<WorkService>, RequestContext, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, Error>> + Send + 'static,
    I: DeserializeOwned + JsonSchema + Clone + Send + Sync + 'static,
    O: Serialize + JsonSchema + Send + 'static,
{
    let svc = Arc::clone(svc);
    let f = Arc::new(f);
    Operation::mutation(
        id,
        method,
        path,
        scope,
        move |ctx: RequestContext, input: I| {
            let (svc, f) = (Arc::clone(&svc), Arc::clone(&f));
            // A command that loses a race to another replica runs again on
            // the state that replica wrote (ADR 0037).
            async move {
                crate::service::retrying(|| f(Arc::clone(&svc), ctx.clone(), input.clone())).await
            }
        },
    )
}

/// Every Work operation.
#[allow(clippy::too_many_lines, reason = "one flat table reads best")]
pub(crate) fn all(s: &Arc<WorkService>) -> Vec<Operation> {
    use HttpMethod::{Delete, Patch, Post, Put};
    use Scope::{Admin, Work};
    vec![
        // Teams.
        query(s, "work.list_teams", "/v1/work/teams", |s, c, i| async move { s.list_teams(c, i).await })
            .summary("Lists teams with their workflows"),
        query(s, "work.get_team", "/v1/work/teams/{team}", |s, c, i| async move { s.get_team(c, i).await })
            .summary("Gets a team by key"),
        mutation(s, "work.create_team", Post, "/v1/work/teams", Admin, |s, c, i| async move {
            s.create_team(c, i).await
        })
        .summary("Creates a team")
        .fresh_auth(FRESH),
        mutation(s, "work.put_team", Put, "/v1/work/teams/{team}", Admin, |s, c, i| async move {
            s.put_team(c, i).await
        })
        .summary("Replaces a team's configuration (team.yaml)")
        .description("Statuses, estimates, cycles, triage rules, SLAs, labels and templates. The file is validated before anything changes.")
        .fresh_auth(FRESH),
        mutation(s, "work.archive_team", Post, "/v1/work/teams/{team}/archive", Admin, |s, c, i| async move {
            s.set_team_archived(c, i, true).await
        })
        .summary("Archives a team")
        .description("The team keeps its issues, keys and history, readable as before, but takes no new issues, moves or triage routes, and leaves team lists unless they ask for archived teams.")
        .fresh_auth(FRESH),
        mutation(s, "work.restore_team", Post, "/v1/work/teams/{team}/restore", Admin, |s, c, i| async move {
            s.set_team_archived(c, i, false).await
        })
        .summary("Restores an archived team")
        .fresh_auth(FRESH),
        query(s, "work.get_team_settings", "/v1/work/teams/{team}/settings", |s, c, i| async move {
            s.get_team_settings(c, i).await
        })
        .summary("Reads a team's whole settings file (team.yaml) as JSON and YAML"),
        query(s, "work.inbox", "/v1/work/teams/{team}/inbox", |s, c, i| async move { s.inbox(c, i).await })
            .summary("Lists a team's triage inbox"),
        query(s, "work.list_cycles", "/v1/work/teams/{team}/cycles", |s, c, i| async move {
            s.list_cycles(c, i).await
        })
        .summary("Lists a team's cycles with burn-up and velocity"),
        // Saved views.
        query(s, "work.list_views", "/v1/work/views", |s, c, i| async move { s.list_views(c, i).await })
            .summary("Lists the saved views the caller can open"),
        mutation(s, "work.create_view", Post, "/v1/work/views", Work, |s, c, i| async move {
            s.create_view(c, i).await
        })
        .summary("Saves a view")
        .description("Private views are the owner's; team views show to the team's members; workspace views to everyone."),
        mutation(s, "work.put_view", Put, "/v1/work/views/{view_id}", Work, |s, c, i| async move {
            s.put_view(c, i).await
        })
        .summary("Replaces a saved view the caller owns"),
        mutation(s, "work.delete_view", Delete, "/v1/work/views/{view_id}", Work, |s, c, i| async move {
            s.delete_view(c, i).await
        })
        .summary("Deletes a saved view the caller owns"),
        query(s, "work.insights", "/v1/work/insights", |s, c, i| async move { s.insights(c, i).await })
            .summary("Measures lead, cycle and triage time, throughput and intake")
            .description("Percentiles (p50, p85, p95) and means in hours over a date range, for all issues or a filter, optionally broken down by team, assignee, priority, label, project, cycle or estimate and split into weeks or months."),
        // Issues.
        query(s, "work.query_issues", "/v1/work/issues", |s, c, i| async move { s.query_issues(c, i).await })
            .summary("Lists issues by filter, grouped and ordered"),
        query(s, "work.get_issue", "/v1/work/issues/{key}", |s, c, i| async move { s.get_issue(c, i).await })
            .summary("Gets an issue by key, old key or ID"),
        mutation(s, "work.create_issue", Post, "/v1/work/issues", Work, |s, c, i| async move {
            s.create_issue(c, i).await
        })
        .summary("Creates an issue")
        .description("Triage rules run first and may route it to another team. Filing from a document block returns the chip to write into the page."),
        mutation(s, "work.update_issue", Patch, "/v1/work/issues/{key}", Work, |s, c, i| async move {
            s.update_issue(c, i).await
        })
        .summary("Changes fields of an issue"),
        mutation(s, "work.move_issue", Post, "/v1/work/issues/{key}/move", Work, |s, c, i| async move {
            s.move_issue(c, i).await
        })
        .summary("Moves an issue to another team; the old key keeps working"),
        mutation(s, "work.triage_issue", Post, "/v1/work/issues/{key}/triage", Work, |s, c, i| async move {
            s.triage_issue(c, i).await
        })
        .summary("Accepts, declines, marks duplicate or snoozes an issue in triage"),
        // Intake forms and suggested triage.
        query(s, "work.list_forms", "/v1/work/teams/{team}/forms", |s, c, i| async move { s.list_forms(c, i).await })
            .summary("Lists a team's intake forms"),
        mutation(s, "work.submit_form", Post, "/v1/work/teams/{team}/forms/{slug}", Work, |s, c, i| async move {
            s.submit_form(c, i).await
        })
        .summary("Submits answers to an intake form as a triage issue"),
        query(s, "work.triage_brief", "/v1/work/issues/{key}/triage/brief", |s, c, i| async move {
            s.triage_brief(c, i).await
        })
        .summary("Gets what an assistant needs to suggest a triage decision")
        .description("The issue, the teams it could go to with their labels and people, open issues with similar titles, and any suggestion already waiting."),
        mutation(s, "work.suggest_triage", Put, "/v1/work/issues/{key}/triage/suggestion", Work, |s, c, i| async move {
            s.suggest_triage(c, i).await
        })
        .summary("Proposes a triage decision for a person to accept")
        .description("The proposal is checked and stored on the issue; nothing else changes until a person accepts it."),
        mutation(s, "work.accept_suggestion", Post, "/v1/work/issues/{key}/triage/suggestion/accept", Work, |s, c, i| async move {
            s.accept_suggestion(c, i).await
        })
        .summary("Applies the waiting triage suggestion as the caller"),
        mutation(s, "work.dismiss_suggestion", Delete, "/v1/work/issues/{key}/triage/suggestion", Work, |s, c, i| async move {
            s.dismiss_suggestion(c, i).await
        })
        .summary("Drops the waiting triage suggestion"),
        // My Issues and My Work.
        query(s, "work.my_issues", "/v1/work/me/issues", |s, c, i| async move { s.my_issues(c, i).await })
            .summary("Lists my issues in focus order"),
        query(s, "work.my_assignments", "/v1/work/me/assignments", |s, c, i| async move {
            s.my_assignments(c, i).await
        })
        .summary("Lists comment threads assigned to me"),
        // Projects.
        query(s, "work.list_projects", "/v1/work/projects", |s, c, i| async move { s.list_projects(c, i).await })
            .summary("Lists projects with progress"),
        query(s, "work.get_project", "/v1/work/projects/{project_id}", |s, c, i| async move {
            s.get_project(c, i).await
        })
        .summary("Gets a project with progress"),
        mutation(s, "work.put_project", Put, "/v1/work/projects/{project_id}", Work, |s, c, i| async move {
            s.put_project(c, i).await
        })
        .summary("Creates or replaces a project page"),
        // Planning: projects from fields, milestones, updates, initiatives,
        // templates (D223).
        mutation(s, "work.create_project", Post, "/v1/work/projects", Work, |s, c, i| async move {
            s.create_project(c, i).await
        })
        .summary("Creates a project from fields"),
        mutation(s, "work.add_milestone", Post, "/v1/work/projects/{project_id}/milestones", Work, |s, c, i| async move {
            s.add_milestone(c, i).await
        })
        .summary("Adds a milestone to a project"),
        query(s, "work.list_updates", "/v1/work/projects/{project_id}/updates", |s, c, i| async move {
            s.list_updates(c, i).await
        })
        .summary("Lists a project's health updates, newest first"),
        mutation(s, "work.post_update", Post, "/v1/work/projects/{project_id}/updates", Work, |s, c, i| async move {
            s.post_update(c, i).await
        })
        .summary("Posts a health update: on track, at risk or off track"),
        query(s, "work.list_initiatives", "/v1/work/initiatives", |s, c, i| async move {
            s.list_initiatives(c, i).await
        })
        .summary("Lists initiatives with their projects' rolled-up health and progress"),
        mutation(s, "work.create_initiative", Post, "/v1/work/initiatives", Work, |s, c, i| async move {
            s.create_initiative(c, i).await
        })
        .summary("Creates an initiative"),
        mutation(s, "work.put_initiative", Put, "/v1/work/initiatives/{initiative_id}", Work, |s, c, i| async move {
            s.put_initiative(c, i).await
        })
        .summary("Replaces an initiative page"),
        query(s, "work.list_templates", "/v1/work/teams/{team}/templates", |s, c, i| async move {
            s.list_templates(c, i).await
        })
        .summary("Lists a team's issue templates in full"),
        // Comments.
        query(s, "work.list_comments", "/v1/work/comments", |s, c, i| async move { s.list_comments(c, i).await })
            .summary("Lists comment threads on an issue, page or block"),
        mutation(s, "work.add_comment", Post, "/v1/work/comments", Work, |s, c, i| async move {
            s.add_comment(c, i).await
        })
        .summary("Starts a thread or replies"),
        mutation(s, "work.edit_comment", Patch, "/v1/work/comments/{comment_id}", Work, |s, c, i| async move {
            s.edit_comment(c, i).await
        })
        .summary("Edits my comment"),
        mutation(s, "work.delete_comment", Delete, "/v1/work/comments/{comment_id}", Work, |s, c, i| async move {
            s.delete_comment(c, i).await
        })
        .summary("Deletes my comment, or any comment as an admin"),
        mutation(s, "work.react", Post, "/v1/work/comments/{comment_id}/reactions", Work, |s, c, i| async move {
            s.react(c, i).await
        })
        .summary("Adds or removes my reaction"),
        mutation(s, "work.resolve_thread", Post, "/v1/work/threads/{thread_id}/resolve", Work, |s, c, i| async move {
            s.resolve_thread(c, i).await
        })
        .summary("Resolves or reopens a thread"),
        mutation(s, "work.assign_thread", Post, "/v1/work/threads/{thread_id}/assign", Work, |s, c, i| async move {
            s.assign_thread(c, i).await
        })
        .summary("Assigns, reassigns or unassigns a thread"),
    ]
}
