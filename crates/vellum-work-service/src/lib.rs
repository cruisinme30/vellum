// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Work service: issue tracking and comments behind the gateway.
//!
//! [`WorkService`] loads the Work files of a workspace (`teams/`,
//! `projects/`, `.comments/`) from its blob store into the `vellum-work`
//! engine, and implements [`vellum_contracts::Service`]. Every engine
//! command and query is an [`vellum_contracts::Operation`], so the gateway
//! serves each one as a REST route, an OpenAPI 3.1 operation and a GraphQL
//! field (`work.create_issue` becomes `mutation { workCreateIssue }`).
//!
//! See ADR 0034 (D135 to D137).

pub mod api;
pub mod journal;
pub mod ports;
pub mod routes;
mod service;

use std::sync::Arc;

use vellum_contracts::{Operation, Service};

pub use ports::{
    Clock, Directory, EventLog, EventSink, FixedClock, StaticDirectory,
    SystemClock,
};
pub use service::embed::Outcome;
pub use service::embeds::{EmbedGroup, EmbedIssue, EmbedResult, EmbedView};
pub use service::live::LiveIssues;
pub use service::{
    DEFAULT_LIST, LoadReport, MAX_LIST, READ_FRESH, WorkService,
};

impl Service for WorkService {
    fn name(&self) -> &'static str {
        "work"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        service::operations(&self)
    }
}
