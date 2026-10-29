// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Work integrations (brief §8).
//!
//! Everything that connects issue tracking to the outside world or derives
//! something from it, as pure domain logic behind ports:
//!
//! | Module | Does |
//! |---|---|
//! | [`git`] | GitHub and GitLab automation: branch names, magic words, per-team state mapping, "all pull requests must merge" |
//! | [`intake`] | Slack, Sentry, email and intake forms create issues in triage |
//! | [`import`] | Linear, Jira and GitHub Issues import |
//! | [`recurring`] | Recurring issues, created at 00:01 team time |
//! | [`create_url`] | The prefilled create-issue URL |
//! | [`insights`] | Cycle time, lead time and triage time |
//! | [`triage`] | AI-suggested triage through the user's own model |
//! | [`receive`] | HTTP receivers for GitHub, GitLab, Slack, Sentry and inbound mail |
//! | [`work`] | The [`model::WorkCommands`] port over the issue tracker engine |
//!
//! The issue core owns issues, states and numbering. This crate reads it
//! through [`model::IssueSnapshot`] and changes it only through the
//! [`model::WorkCommands`] port, so it never touches another service's
//! tables (ADR 0017, D79).

pub mod create_url;
pub mod git;
pub mod import;
pub mod inbound;
pub mod insights;
pub mod intake;
pub mod model;
pub mod receive;
pub mod recurring;
pub mod triage;
pub mod work;

mod mac;
