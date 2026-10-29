// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Request-level defences from the brief's security section (§11).
//!
//! - [`ssrf`]: outbound HTTP that can only reach the public internet, for
//!   link previews, embeds, webhooks and imports.
//! - [`e2e`]: member key registry and server-feature policy for
//!   end-to-end encrypted private spaces.
//! - [`rate`]: keyed rate limits and sign-in lockout.
//! - [`secrets`]: secret scanning on save.
//! - [`upload`]: upload sniffing, size limits, image re-encoding and the
//!   virus-scanner hook.

pub mod e2e;
pub mod rate;
pub mod secrets;
pub mod ssrf;
pub mod upload;
