// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Adapters between the identity service and the edge: the gateway's
//! [`Authenticator`] and the SSRF-guarded client-metadata fetcher.

use std::sync::Arc;
use std::time::Duration;

use vellum_api::auth::{Authenticated, Authenticator, RequestMeta};
use vellum_contracts::BoxFuture;
use vellum_identity::http::cookie;
use vellum_identity::{ClientMetadataFetcher, Identity};
use vellum_kernel::{Error, ErrorKind};
use vellum_security::ssrf::{SafeClient, UrlPolicy};

/// Authenticates gateway requests with identity's credentials.
///
/// - `Authorization: Bearer` takes an API token or an `OAuth` access
///   token; an invalid one is refused with 401.
/// - Otherwise the session cookie; an unknown or expired cookie leaves the
///   caller anonymous, so a stale cookie never blocks signing in again.
#[derive(Debug, Clone)]
pub struct IdentityAuthenticator {
    identity: Arc<Identity>,
}

impl IdentityAuthenticator {
    /// Wraps the identity service.
    pub const fn new(identity: Arc<Identity>) -> Self {
        Self { identity }
    }
}

impl Authenticator for IdentityAuthenticator {
    fn authenticate<'a>(
        &'a self,
        request: &'a RequestMeta,
    ) -> BoxFuture<'a, Result<Authenticated, Error>> {
        Box::pin(async move {
            let bearer = request
                .headers
                .get(http::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| {
                    v.strip_prefix("Bearer ")
                        .or_else(|| v.strip_prefix("bearer "))
                });
            if let Some(token) = bearer {
                let resolved = self.identity.bearer(token.trim())?;
                return Ok(Authenticated {
                    principal: resolved.principal,
                    csrf_binding: None,
                });
            }
            let name = self.identity.config().cookie_name();
            Ok(cookie(&request.headers, name)
                .and_then(|value| self.identity.session(value))
                .map_or_else(Authenticated::anonymous, |resolved| Authenticated {
                    principal: resolved.principal,
                    csrf_binding: resolved.csrf_binding,
                }))
        })
    }
}

/// The largest Client ID Metadata Document accepted.
const MAX_CLIENT_DOCUMENT: u64 = 64 * 1024;

/// Fetches `OAuth` Client ID Metadata Documents through the SSRF guard:
/// public `https` addresses only, no redirects, 64 `KiB`, 5 seconds.
#[derive(Debug, Clone)]
pub struct GuardedFetcher {
    client: SafeClient,
}

impl GuardedFetcher {
    /// Builds the fetcher.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the TLS backend cannot start.
    pub fn new() -> Result<Self, Error> {
        let mut policy = UrlPolicy::default();
        policy.allow_http = false;
        policy.allowed_ports = vec![443];
        policy.max_redirects = 0;
        policy.timeout = Duration::from_secs(5);
        policy.max_body_bytes = MAX_CLIENT_DOCUMENT;
        let client = SafeClient::new(policy)
            .map_err(|e| Error::new(ErrorKind::Internal, format!("HTTP client: {e}")))?;
        Ok(Self { client })
    }
}

impl ClientMetadataFetcher for GuardedFetcher {
    fn fetch<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>, Error>> {
        Box::pin(async move {
            self.client.get_bytes(url).await.map_err(|e| {
                Error::new(
                    ErrorKind::InvalidInput,
                    format!("cannot fetch the client metadata document: {e}"),
                )
            })
        })
    }
}
