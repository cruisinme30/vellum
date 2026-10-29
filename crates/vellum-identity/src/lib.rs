// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The identity service: who may sign in, and with what.
//!
//! - **Accounts** with Argon2id passwords, created by the first-run set-up
//!   or by an administrator.
//! - **Cookie sessions**: an opaque `HttpOnly`, `SameSite=Lax` cookie
//!   (`__Host-vellum_session` over HTTPS), idle and absolute expiry, and a
//!   CSRF binding for the gateway.
//! - **Lockout**: exponential back-off per address and per client IP.
//! - **Re-authentication**: admin operations need a sign-in in the last
//!   15 minutes ([`service::FRESH_AUTH`]).
//! - **API tokens** (`vlm_…`) with chosen scopes, hashed at rest.
//! - **`OAuth` 2.1 authorization server** for MCP clients and the desktop
//!   app ([`oauth`]).
//! - **Break-glass**: a one-time code written to the data directory by
//!   `vellum admin break-glass` opens a 15-minute administrator session.
//!
//! The JSON operations register through [`service::IdentityService`];
//! cookie and redirect endpoints are routers in [`http`]. Single sign-on
//! (OIDC and SAML) arrives in a later change.

pub mod http;
pub mod identity;
pub mod oauth;
pub mod password;
pub mod secret;
pub mod service;
pub mod store;

pub use identity::{
    BREAK_GLASS_EMAIL, Identity, IdentityConfig, NewSession, Resolved, TokenInfo, UserView,
};
pub use oauth::{ClientMetadataFetcher, DESKTOP_CLIENT_ID, NoMetadataFetch};
pub use service::IdentityService;
pub use store::Store;
