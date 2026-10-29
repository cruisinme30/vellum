// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! GitHub and GitLab automation.
//!
//! ```text
//! provider webhook ──► github / gitlab ──► GitEvent ──► refs (keys, magic words)
//!                       (verify, parse)                      │
//!                                         links (per issue) ◄┘
//!                                                │
//!                       automation (team mapping, all PRs must merge)
//!                                                │
//!                                   WorkCommands::set_state
//! ```
//!
//! [`branch`] also generates the branch name and pull request title that
//! the app copies with `⌘⇧.`.

pub mod automation;
pub mod branch;
pub mod event;
pub mod github;
pub mod gitlab;
pub mod refs;
pub mod service;

pub use automation::{Mapping, PrLink, PrStatus, Stage};
pub use event::{GitEvent, PullRequestAction};
pub use refs::{RefKind, Reference};
