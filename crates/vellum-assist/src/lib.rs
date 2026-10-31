// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Bring-your-own-model features (R-AI-8, R-SR-5): summarise a page, ask a
//! space and get an answer with citations, and draft a page from a
//! template, all with the user's own model and API key.
//!
//! Vellum runs no model. The browser keeps the key and sends it with each
//! request; the server reads the workspace as the user, calls the model
//! through the SSRF guard and forgets the key (ADR 0019, D238-D240).

pub mod model;
pub mod prompt;
pub mod service;
pub mod sources;

pub use model::{ApiKey, HttpModel, ModelClient, ModelSettings, Provider};
pub use service::AssistService;
pub use sources::Sources;
