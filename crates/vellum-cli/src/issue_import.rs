// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Importing issues from Linear, Jira and GitHub Issues (#151), mounted
//! at [`PREFIX`] with the upload body limit.
//!
//! `POST ?tracker=linear|jira|github&team=ENG[&repo=owner/name]&name=…[&dry_run=true]`
//! takes the tracker's export as the body: Linear's CSV, Jira search
//! results as JSON, or GitHub's issue list as JSON. The file goes through
//! [`upload::check`] like any upload. The importers in
//! `vellum_integrations::import` read it and plan it onto the team:
//! states by name then category, people by email, name or handle. A dry
//! run answers the plan; otherwise the issues are created oldest first in
//! one Work transaction, as the caller, and parents and relations are set
//! in a second one. Issues imported before (matched by their source link)
//! are skipped, so an import can be run again.
//!
//! Only workspace admins with the `work` scope may import: an import
//! creates many issues and attributes them to other people.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::post;
use serde::Serialize;
use vellum_contracts::Scope;
use vellum_identity::Identity;
use vellum_integrations::import::{self, Export, Member, Warning};
use vellum_integrations::model::{IssueKey, WorkError};
use vellum_integrations::work::steps;
use vellum_kernel::{Actor, Error, ErrorKind};
use vellum_security::upload::{self, Kind, Scanner, UploadPolicy};
use vellum_work::UserRef;
use vellum_work_service::WorkService;

use crate::uploads::{caller, invalid, json, refused, request_id, respond};

/// Where the issue import route is mounted.
pub const PREFIX: &str = "/api/v1/issue-import";

/// What the route shares.
struct IssueImport<S> {
    work: Arc<WorkService>,
    identity: Arc<Identity>,
    scanner: Option<Arc<S>>,
    policy: UploadPolicy,
}

/// The issue import route.
pub fn router<S: Scanner + 'static>(
    work: Arc<WorkService>,
    identity: Arc<Identity>,
    scanner: Option<Arc<S>>,
    policy: UploadPolicy,
) -> Router {
    Router::new()
        .route("/", post(import_issues::<S>))
        .with_state(Arc::new(IssueImport {
            work,
            identity,
            scanner,
            policy,
        }))
}

/// The route when `vellum serve` runs without a workspace: 503.
pub fn unavailable_router() -> Router {
    Router::new().fallback(|request: Request| async move {
        respond(
            request_id(&request),
            Error::new(
                ErrorKind::Unavailable,
                "issue import needs a workspace index",
            ),
        )
    })
}

/// What an import did, or would do on a dry run.
#[derive(Debug, Serialize)]
struct ImportReport {
    /// The source tracker.
    tracker: String,
    /// The team the issues go to.
    team: String,
    /// Issues read from the file.
    read: usize,
    /// Issues created (on a dry run: that would be created).
    created: usize,
    /// Issues skipped because an earlier import created them.
    skipped: usize,
    /// Parents and relations set.
    links: usize,
    /// The new issues' keys by source ID; empty on a dry run.
    keys: Vec<(String, String)>,
    /// What could not be carried over exactly.
    warnings: Vec<Warning>,
    dry_run: bool,
}

async fn import_issues<S: Scanner + 'static>(
    State(state): State<Arc<IssueImport<S>>>,
    request: Request,
) -> Response {
    let id = request_id(&request);
    let result = async {
        let Actor::User { user } = caller(&request, Scope::Work)? else {
            return Err(forbidden());
        };
        let me = state.identity.user(user)?;
        if !me.admin {
            return Err(forbidden());
        }
        let handle = UserRef::parse(&me.handle).map_err(|_| forbidden())?;
        let query: std::collections::HashMap<String, String> =
            url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
                .into_owned()
                .collect();
        let tracker = query.get("tracker").cloned().unwrap_or_default();
        if !matches!(tracker.as_str(), "linear" | "jira" | "github") {
            return Err(invalid("tracker must be linear, jira or github"));
        }
        let team = query
            .get("team")
            .filter(|t| !t.is_empty())
            .cloned()
            .ok_or_else(|| invalid("say the team key with `team`"))?;
        let repo = query.get("repo").cloned().unwrap_or_default();
        if tracker == "github" && repo.is_empty() {
            return Err(invalid("a GitHub import needs `repo` (owner/name)"));
        }
        let name = query
            .get("name")
            .filter(|n| !n.is_empty())
            .cloned()
            .ok_or_else(|| invalid("say the file's name in the `name` query parameter"))?;
        let dry_run = query.get("dry_run").is_some_and(|v| v == "true");
        // RT-045: the body is held in memory, so reserve it first.
        let _reserved = crate::uploads::reserve_body(
            &crate::uploads::Budget::shared(),
            &request,
            user,
            state.policy.max_bytes,
        )?;
        let body = axum::body::to_bytes(request.into_body(), usize::MAX)
            .await
            .map_err(|_| invalid("the upload did not arrive whole"))?;
        let cleaned = upload::check(
            state.policy,
            state.scanner.as_deref(),
            name,
            Vec::from(body),
        )
        .await
        .map_err(|e| refused(&e))?;
        if cleaned.kind != Kind::Text {
            return Err(invalid("an issue export is a CSV or JSON text file"));
        }
        let text = String::from_utf8(cleaned.bytes)
            .map_err(|_| invalid("the export is not UTF-8 text"))?;
        let export = read(&tracker, &text, &repo)?;
        let members = members(&state.identity);
        run(
            &state.work,
            handle,
            &tracker,
            &team,
            &export,
            &members,
            dry_run,
        )
        .await
    }
    .await;
    result.map_or_else(|e| respond(id, e), |report| json(status(&report), &report))
}

const fn status(report: &ImportReport) -> StatusCode {
    if report.dry_run {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    }
}

/// Reads the export with the tracker's reader.
fn read(tracker: &str, text: &str, repo: &str) -> Result<Export, Error> {
    let read = match tracker {
        "linear" => import::linear::read(text),
        "jira" => import::jira::read(text, None),
        _ => import::github::read(text, repo),
    };
    read.map_err(|e| Error::new(ErrorKind::InvalidInput, e.to_string()))
}

/// The workspace's people, for matching assignees and reporters.
fn members(identity: &Identity) -> Vec<Member> {
    identity
        .users()
        .into_iter()
        .filter(|u| !u.disabled)
        .map(|u| Member {
            id: u.handle.clone(),
            email: u.email,
            name: u.name,
            handles: vec![u.handle],
        })
        .collect()
}

/// Plans `export` onto `team` and, unless `dry_run`, creates it as `actor`.
async fn run(
    work: &WorkService,
    actor: UserRef,
    tracker: &str,
    team: &str,
    export: &Export,
    members: &[Member],
    dry_run: bool,
) -> Result<ImportReport, Error> {
    let (states, earlier) = work
        .read(|graph, _| (steps::states(graph, team), steps::imported(graph, tracker)))
        .await;
    let states = states.map_err(|e| work_error(&e))?;
    let already: BTreeSet<String> = earlier.keys().cloned().collect();
    let plan = import::plan(tracker, export, team, &states, members, &already);
    let skipped = export
        .issues
        .iter()
        .filter(|i| already.contains(&i.external_id))
        .count();
    let mut report = ImportReport {
        tracker: tracker.to_owned(),
        team: team.to_owned(),
        read: export.issues.len(),
        created: plan.create.len(),
        skipped,
        links: 0,
        keys: Vec::new(),
        warnings: plan.warnings.clone(),
        dry_run,
    };
    if dry_run || plan.create.is_empty() {
        return Ok(report);
    }
    let created = work
        .transact(actor.clone(), |graph, _, ctx| {
            let (keys, changes) =
                steps::import_plan(graph, &plan, ctx).map_err(|e| work_error(&e))?;
            Ok((changes, None, keys))
        })
        .await?;
    let mut all = earlier;
    all.extend(created.clone());
    let (links, warnings) = import::resolve_links(export, &all);
    report.warnings.extend(warnings);
    if !links.is_empty() {
        work.transact(actor, |graph, _, ctx| {
            let changes = steps::apply_links(graph, &links, ctx).map_err(|e| work_error(&e))?;
            Ok((changes, None, ()))
        })
        .await?;
    }
    report.links = links.len();
    report.keys = created
        .into_iter()
        .map(|(external, key): (String, IssueKey)| (external, key.to_string()))
        .collect();
    tracing::info!(
        tracker,
        team,
        created = report.created,
        links = report.links,
        "issues imported"
    );
    Ok(report)
}

fn forbidden() -> Error {
    Error::new(
        ErrorKind::PermissionDenied,
        "only workspace admins may import issues",
    )
}

fn work_error(e: &WorkError) -> Error {
    match e {
        WorkError::NotFound(what) => Error::new(ErrorKind::NotFound, format!("not found: {what}")),
        WorkError::Refused(why) => Error::new(ErrorKind::InvalidInput, why.clone()),
        WorkError::Unavailable(why) => Error::new(ErrorKind::Unavailable, why.clone()),
    }
}
