// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Intake forms and suggested triage, served over the gateway.
//!
//! People see a team's forms and submit answers as triage issues. An
//! assistant reads a triage brief, proposes a decision, and a person
//! accepts or dismisses it; the proposal changes nothing on its own.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::instrument;
use vellum_contracts::RequestContext;
use vellum_kernel::Error;
use vellum_work::intake::{FieldKind, IntakeForm, MapsTo};
use vellum_work::suggest::{TriageBrief, TriageSuggestion};
use vellum_work::{IssueId, TeamKey, UserRef, WorkGraph};

use super::{WorkService, priority};
use crate::api::{IssueOut, IssueRef, TeamRef, invalid, name_of, rfc3339};

/// One question on a form.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FieldOut {
    /// The answer's key in a submission.
    pub key: String,
    /// The question.
    pub label: String,
    /// `text`, `long_text`, `select` or `email`.
    pub kind: String,
    /// The choices of a `select` question.
    pub options: Vec<String>,
    /// Whether an answer is needed.
    pub required: bool,
    /// Where the answer goes: `description`, `title`, `priority` or `label`.
    pub maps_to: String,
}

/// An intake form.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FormOut {
    /// The form's slug, unique in its team.
    pub slug: String,
    /// The form's title, such as "Request access".
    pub title: String,
    /// Help shown above the questions.
    pub description: String,
    /// Whether people outside the workspace may submit through a share link.
    pub public: bool,
    /// The questions, in order.
    pub fields: Vec<FieldOut>,
}

impl FormOut {
    fn of(form: &IntakeForm) -> Self {
        Self {
            slug: form.slug.clone(),
            title: form.title.clone(),
            description: form.description.clone(),
            public: form.public,
            fields: form
                .fields
                .iter()
                .map(|f| {
                    let (kind, options) = match &f.kind {
                        FieldKind::Text => ("text", Vec::new()),
                        FieldKind::LongText => ("long_text", Vec::new()),
                        FieldKind::Select { options } => ("select", options.clone()),
                        FieldKind::Email => ("email", Vec::new()),
                    };
                    FieldOut {
                        key: f.key.clone(),
                        label: f.label.clone(),
                        kind: kind.to_owned(),
                        options,
                        required: f.required,
                        maps_to: name_of::<MapsTo>(&f.maps_to),
                    }
                })
                .collect(),
        }
    }
}

/// A team's intake forms.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct FormsOut {
    /// The forms, in the order the team lists them.
    pub forms: Vec<FormOut>,
}

/// Answers to a form.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SubmitForm {
    /// The team's key.
    pub team: String,
    /// The form's slug.
    pub slug: String,
    /// Answers by question key.
    #[serde(default)]
    pub answers: BTreeMap<String, String>,
}

/// A team an issue could go to, in a triage brief.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TeamOptionOut {
    /// The team key.
    pub key: String,
    /// The team name.
    pub name: String,
    /// Labels usable on the team's issues.
    pub labels: Vec<String>,
    /// People who can be assigned.
    pub people: Vec<String>,
}

/// An open issue with a similar title.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SimilarOut {
    /// Its identifier, such as `ENG-142`.
    pub key: String,
    /// Its title.
    pub title: String,
    /// Shared title words, in percent.
    pub overlap: u8,
}

/// A proposed triage decision.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SuggestionOut {
    /// The team to move the issue to, if not its current one.
    pub team: Option<String>,
    /// The priority, 0 (none) to 4 (low).
    pub priority: Option<u8>,
    /// Labels to add.
    pub labels: Vec<String>,
    /// Who should own the issue.
    pub assignee: Option<String>,
    /// The issue this one repeats.
    pub duplicate_of: Option<String>,
    /// Why, in a sentence or two.
    pub reason: String,
    /// Which assistant made it.
    pub model: String,
    /// When it was made (RFC 3339).
    pub at: String,
}

impl SuggestionOut {
    fn of(s: &TriageSuggestion, graph: &WorkGraph) -> Self {
        Self {
            team: s.team.as_ref().map(TeamKey::to_string),
            priority: s.priority.map(vellum_work::Priority::number),
            labels: s.labels.clone(),
            assignee: s.assignee.as_ref().map(UserRef::to_string),
            duplicate_of: s.duplicate_of.map(|id| {
                graph
                    .issue(&id)
                    .map_or_else(|| id.to_string(), |i| i.key().to_string())
            }),
            reason: s.reason.clone(),
            model: s.model.clone(),
            at: rfc3339(s.at),
        }
    }
}

/// What an assistant reads before suggesting a triage decision.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct BriefOut {
    /// The issue's identifier.
    pub key: String,
    /// Its title.
    pub title: String,
    /// The start of its description.
    pub body: String,
    /// The team whose inbox holds it.
    pub team: String,
    /// Where it could go.
    pub teams: Vec<TeamOptionOut>,
    /// Open issues with similar titles, closest first.
    pub similar: Vec<SimilarOut>,
    /// The suggestion already waiting, if any.
    pub suggestion: Option<SuggestionOut>,
}

impl BriefOut {
    fn of(brief: TriageBrief, suggestion: Option<SuggestionOut>) -> Self {
        Self {
            key: brief.key,
            title: brief.title,
            body: brief.body,
            team: brief.team.to_string(),
            teams: brief
                .teams
                .into_iter()
                .map(|t| TeamOptionOut {
                    key: t.key.to_string(),
                    name: t.name,
                    labels: t.labels,
                    people: t.people.iter().map(UserRef::to_string).collect(),
                })
                .collect(),
            similar: brief
                .similar
                .into_iter()
                .map(|s| SimilarOut {
                    key: s.key,
                    title: s.title,
                    overlap: s.overlap,
                })
                .collect(),
            suggestion,
        }
    }
}

/// A proposed triage decision for an issue.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SuggestTriage {
    /// The issue.
    pub key: String,
    /// The team to move it to.
    pub team: Option<String>,
    /// The priority, 0 (none) to 4 (low).
    pub priority: Option<u8>,
    /// Labels to add.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Who should own it (a handle).
    pub assignee: Option<String>,
    /// The key of an open issue it repeats.
    pub duplicate_of: Option<String>,
    /// Why, in a sentence or two; shown next to the suggestion.
    pub reason: String,
    /// Which assistant is suggesting, as it names itself.
    #[serde(default)]
    pub model: String,
}

impl WorkService {
    fn form(graph: &WorkGraph, team: &TeamKey, slug: &str) -> Result<IntakeForm, Error> {
        graph
            .team(team)
            .and_then(|t| t.forms.iter().find(|f| f.slug == slug))
            .cloned()
            .ok_or_else(|| {
                Error::new(
                    vellum_kernel::ErrorKind::NotFound,
                    format!("team {team} has no form {slug}"),
                )
            })
    }

    pub(crate) async fn list_forms(
        &self,
        _rc: RequestContext,
        i: TeamRef,
    ) -> Result<FormsOut, Error> {
        let st = self.state.lock().await;
        let key = Self::team_key(&st.graph, &i.team)?;
        let forms = st
            .graph
            .team(&key)
            .map(|t| t.forms.iter().map(FormOut::of).collect())
            .unwrap_or_default();
        Ok(FormsOut { forms })
    }

    #[instrument(skip_all, fields(team = %i.team, form = %i.slug))]
    pub(crate) async fn submit_form(
        &self,
        rc: RequestContext,
        i: SubmitForm,
    ) -> Result<IssueOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let submitter = ctx.actor.as_ref().map(|u| format!("@{u}"));
        let mut st = self.state.lock().await;
        let team = Self::team_key(&st.graph, &i.team)?;
        let _ = Self::form(&st.graph, &team, &i.slug)?;
        let id = IssueId::generate();
        let backup = st.clone();
        let changes =
            st.graph
                .submit_form(&team, &i.slug, id, &i.answers, submitter.as_deref(), &ctx)?;
        self.commit(
            &mut st,
            backup,
            changes,
            None,
            Vec::new(),
            ctx.actor.as_ref(),
        )
        .await?;
        self.issue_out(&st, &id, ctx.now)
    }

    pub(crate) async fn triage_brief(
        &self,
        _rc: RequestContext,
        i: IssueRef,
    ) -> Result<BriefOut, Error> {
        let st = self.state.lock().await;
        let id = Self::issue_id(&st.graph, &i.key)?;
        let brief = st.graph.triage_brief(&id)?;
        let waiting = st
            .graph
            .suggestion(&id)
            .map(|s| SuggestionOut::of(&s, &st.graph));
        Ok(BriefOut::of(brief, waiting))
    }

    #[instrument(skip_all, fields(issue = %i.key))]
    pub(crate) async fn suggest_triage(
        &self,
        rc: RequestContext,
        i: SuggestTriage,
    ) -> Result<SuggestionOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.state.lock().await;
        let id = Self::issue_id(&st.graph, &i.key)?;
        let suggestion = TriageSuggestion {
            team: i
                .team
                .as_deref()
                .map(|t| Self::team_key(&st.graph, t))
                .transpose()?,
            priority: i.priority.map(priority).transpose()?,
            labels: i.labels,
            assignee: i.assignee.as_deref().map(UserRef::parse).transpose()?,
            duplicate_of: i
                .duplicate_of
                .as_deref()
                .map(|k| Self::issue_id(&st.graph, k))
                .transpose()?,
            reason: i.reason,
            model: i.model,
            at: ctx.now,
        };
        let backup = st.clone();
        let changes = st.graph.suggest_triage(&id, suggestion, &ctx)?;
        self.commit(
            &mut st,
            backup,
            changes,
            None,
            Vec::new(),
            ctx.actor.as_ref(),
        )
        .await?;
        st.graph
            .suggestion(&id)
            .map(|s| SuggestionOut::of(&s, &st.graph))
            .ok_or_else(|| invalid("the suggestion was not stored"))
    }

    #[instrument(skip_all, fields(issue = %i.key))]
    pub(crate) async fn accept_suggestion(
        &self,
        rc: RequestContext,
        i: IssueRef,
    ) -> Result<IssueOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.state.lock().await;
        let id = Self::issue_id(&st.graph, &i.key)?;
        let backup = st.clone();
        let changes = st.graph.accept_suggestion(&id, &ctx)?;
        self.commit(
            &mut st,
            backup,
            changes,
            None,
            Vec::new(),
            ctx.actor.as_ref(),
        )
        .await?;
        self.issue_out(&st, &id, ctx.now)
    }

    pub(crate) async fn dismiss_suggestion(
        &self,
        rc: RequestContext,
        i: IssueRef,
    ) -> Result<IssueOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let mut st = self.state.lock().await;
        let id = Self::issue_id(&st.graph, &i.key)?;
        let backup = st.clone();
        let changes = st.graph.dismiss_suggestion(&id)?;
        self.commit(
            &mut st,
            backup,
            changes,
            None,
            Vec::new(),
            ctx.actor.as_ref(),
        )
        .await?;
        self.issue_out(&st, &id, ctx.now)
    }
}
