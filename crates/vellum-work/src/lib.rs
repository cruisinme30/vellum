// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Work: Linear-class issue tracking where every issue is a file.
//!
//! This crate is the engine behind teams, issues, cycles, triage, SLAs,
//! projects, initiatives, views and My Issues (brief §8, R-WK-1 to R-WK-5,
//! R-WK-8, R-WK-9, R-WK-11, R-WK-12). It is pure domain logic:
//!
//! - **Files are the truth.** Teams are `teams/KEY/team.yaml`, issues are
//!   `teams/KEY/issues/KEY-n.md`, cycles are `teams/KEY/cycles/*.yaml`,
//!   projects and initiatives are pages. Every type here parses and writes
//!   its file; the index is rebuilt by loading them into a [`WorkGraph`].
//! - **Commands return changes.** [`WorkGraph`] commands validate, apply,
//!   and return [`Changes`]: the files to write and the [`WorkEvent`]s to
//!   publish. Storage, authorization and transport live in the service
//!   layer.
//! - **Categories, not schemes.** Statuses are free; the seven
//!   [`Category`] values drive every view, automation and metric (D15).
//!
//! See ADR 0016 (D76 to D78) and ADR 0033 (D129 to D131) for the
//! decisions this crate records.

pub mod comment;
pub mod cycle;
pub mod doclink;
pub mod embed;
pub mod error;
pub mod event;
pub mod filter;
pub mod graph;
pub mod id;
pub mod insights;
pub mod intake;
pub mod issue;
pub mod label;
pub mod mention;
pub mod my_issues;
pub mod project;
pub mod scale;
pub mod sla;
pub mod suggest;
pub mod team;
pub mod template;
pub mod timestamp;
pub mod triage;
pub mod view;
pub mod workflow;

pub use comment::{BlockRef, Comment, CommentChanges, Comments, NewComment, Target, Thread};
pub use cycle::{Cycle, CycleKey, CycleSettings};
pub use doclink::{BackLink, IssueEmbed};
pub use error::{Result, WorkError};
pub use event::{IssueSnapshot, WorkEvent};
pub use filter::{Condition, Field, Filter, Op};
pub use graph::{Changes, Ctx, IssuePatch, NewIssue, WorkGraph};
pub use id::{InitiativeId, IssueId, MilestoneId, ProjectId, TeamId, UpdateId, UserRef, ViewId};
pub use intake::IntakeForm;
pub use issue::{Issue, IssueKey, Relation, RelationKind};
pub use project::{Health, Initiative, Project, ProjectUpdate};
pub use scale::{Estimate, EstimateScale, Priority};
pub use suggest::{TriageBrief, TriageSuggestion};
pub use team::{Team, TeamKey};
pub use triage::TriageAction;
pub use view::View;
pub use workflow::{Category, Status, StatusKey, Workflow};
