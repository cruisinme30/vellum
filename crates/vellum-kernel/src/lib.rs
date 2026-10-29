// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Shared foundations for every Vellum crate.
//!
//! The kernel sits at the bottom of the crate graph and depends on no other
//! Vellum crate. Today it carries the observability baseline
//! ([`telemetry`]); typed IDs, errors, events and configuration join it in
//! the foundation workstream.

pub mod telemetry;
