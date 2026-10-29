// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Resolving a request's credential into a [`Principal`].
//!
//! The identity service implements [`Authenticator`]; the gateway calls it
//! once per request and stores the result in the request extensions, where
//! handlers and mounted routers find it.

use std::net::IpAddr;

use http::HeaderMap;
use vellum_contracts::{BoxFuture, Principal};
use vellum_kernel::Error;

/// What the authenticator sees of a request.
#[derive(Debug, Clone)]
pub struct RequestMeta {
    /// The request headers (`Authorization`, `Cookie`, `User-Agent`).
    pub headers: HeaderMap,
    /// The peer address, after trusted-proxy handling.
    pub client_ip: Option<IpAddr>,
    /// The request path, for credentials scoped to a path.
    pub path: String,
}

/// The outcome of authentication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Authenticated {
    /// Who is calling.
    pub principal: Principal,
    /// For cookie sessions, the value CSRF tokens are bound to (the stored
    /// session hash). `None` for bearer tokens, which carry no ambient
    /// authority and need no CSRF token.
    pub csrf_binding: Option<[u8; 32]>,
}

impl Authenticated {
    /// The anonymous caller.
    #[must_use]
    pub const fn anonymous() -> Self {
        Self {
            principal: Principal::anonymous(),
            csrf_binding: None,
        }
    }
}

/// Turns credentials into a principal.
pub trait Authenticator: Send + Sync + 'static {
    /// Authenticates a request. Returns the anonymous principal when no
    /// credential is presented, and an `unauthenticated` error when one is
    /// presented but invalid, expired or revoked.
    fn authenticate<'a>(
        &'a self,
        request: &'a RequestMeta,
    ) -> BoxFuture<'a, Result<Authenticated, Error>>;
}

/// An authenticator that treats every caller as anonymous. Used when the
/// identity service is not running (tests, `vellum serve --no-identity`).
#[derive(Debug, Clone, Copy, Default)]
pub struct AnonymousOnly;

impl Authenticator for AnonymousOnly {
    fn authenticate<'a>(
        &'a self,
        _request: &'a RequestMeta,
    ) -> BoxFuture<'a, Result<Authenticated, Error>> {
        Box::pin(async { Ok(Authenticated::anonymous()) })
    }
}
