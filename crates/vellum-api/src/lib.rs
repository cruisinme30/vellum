// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The HTTP gateway.

pub mod auth;
pub mod breaker;
pub mod config;
pub mod deadline;
pub mod error;
pub mod gateway;
pub mod graphql;
pub mod idempotency;
pub mod input;
pub mod openapi;
pub mod public;
pub mod registry;
pub mod request_id;
pub mod security;
pub mod server;
pub mod typescript;
