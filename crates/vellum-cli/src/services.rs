// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The composition root: every service the binary serves.
//!
//! Each service workstream adds its service here (and its raw routers in
//! [`mount`]), so `vellum serve`, the OpenAPI document and the `GraphQL`
//! schema all see the same set.

use std::sync::Arc;

use vellum_api::config::GatewayConfig;
use vellum_api::gateway::{Gateway, GatewayBuilder};
use vellum_contracts::Service;
use vellum_kernel::Error;

/// The binary's version, shown in `/healthz` and the API documents.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Every service, in registration order.
pub fn all() -> Vec<Arc<dyn Service>> {
    Vec::new()
}

/// Adds every service and raw router to a gateway builder.
pub fn mount(mut builder: GatewayBuilder) -> GatewayBuilder {
    for service in all() {
        builder = builder.service(service);
    }
    builder
}

/// A gateway built only to describe the API (no listener, no storage).
pub fn contract_gateway() -> Result<Gateway, Error> {
    mount(GatewayBuilder::new(
        GatewayConfig::new("http://localhost", vec![0; 32]),
        VERSION,
    ))
    .build()
}
