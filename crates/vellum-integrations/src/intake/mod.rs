// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Intake: issues that arrive from outside the app.
//!
//! Every intake channel produces an [`IssueDraft`](crate::model::IssueDraft)
//! with `state: None`, which the issue core files in the team's triage
//! inbox (or backlog when the team has triage off). Each draft carries its
//! [`Source`](crate::model::Source), whose external ID lets the core drop
//! duplicates when a provider retries.
//!
//! | Channel | Authenticated by | Module |
//! |---|---|---|
//! | Slack slash command and message action | `X-Slack-Signature` (HMAC, 5-minute window) | [`slack`] |
//! | Sentry issue alerts | `Sentry-Hook-Signature` (HMAC) | [`sentry`] |
//! | Email to `ENG@intake…` / replies to notifications | the mail server's SPF/DKIM verdict, plus a signed reply token | [`email`] |
//! | Intake forms | the form's own access setting | [`form`] |
//!
//! Text from all of them reaches the description through [`text::Body`],
//! which keeps it as plain text so a sender cannot add links, images,
//! mentions or containers.

pub mod email;
pub mod form;
pub mod sentry;
pub mod slack;
pub mod text;

/// Where intake lands when the sender does not name a team.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routing {
    /// The team used when none is named.
    pub default_team: String,
    /// Teams that accept intake.
    pub teams: std::collections::BTreeSet<String>,
}

impl Routing {
    /// The team named by `word` if it accepts intake.
    #[must_use]
    pub fn team(&self, word: &str) -> Option<String> {
        let upper = word.to_ascii_uppercase();
        self.teams.contains(&upper).then_some(upper)
    }
}
