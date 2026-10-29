// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Public API types shared by the server, the clients and the MCP server.
//!
//! - [`blocks`]: the block model, generated from `schema/blocks.json`.
//! - [`service`]: the seam every service implements so the gateway can
//!   serve it over REST, OpenAPI and GraphQL.
//! - [`principal`]: the authenticated caller and its token scopes.
//! - [`problem`]: the error body every API returns.

pub mod blocks;
pub mod principal;
pub mod problem;
pub mod service;

pub use principal::{CredentialKind, Principal, REAUTHENTICATION_REQUIRED, Scope, ScopeSet};
pub use problem::ApiError;
pub use service::{
    BoxFuture, Handler, HttpMethod, Operation, OperationKind, Readiness, RequestContext, Service,
};
