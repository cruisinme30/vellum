// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Adapters between the identity service and the edge: the gateway's
//! [`Authenticator`], the SSRF-guarded client-metadata fetcher, and the
//! single sign-on settings and HTTP client.

use std::sync::Arc;
use std::time::Duration;

use url::Url;
use vellum_api::auth::{Authenticated, Authenticator, RequestMeta};
use vellum_contracts::BoxFuture;
use vellum_identity::http::cookie;
use vellum_identity::{ClientMetadataFetcher, Identity, SsoConfig, SsoHttp};
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

/// Whether `path` is the MCP endpoint or below it.
/// SCIM checks its own token (ADR 0069, D294), which is no API token.
fn is_scim(path: &str) -> bool {
    path.strip_prefix(vellum_identity::scim::SCIM_PREFIX)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

fn is_mcp(path: &str) -> bool {
    path.strip_prefix("/mcp")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
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
            if bearer.is_some() && is_scim(&request.path) {
                return Ok(Authenticated::anonymous());
            }
            if let Some(token) = bearer {
                let resolved = match self.identity.bearer(token.trim()) {
                    Ok(resolved) => resolved,
                    // `/mcp` answers a bad token itself, with the
                    // `WWW-Authenticate` challenge MCP clients follow to
                    // sign in again (ADR 0019, D217).
                    Err(_) if is_mcp(&request.path) => {
                        return Ok(Authenticated::anonymous());
                    }
                    Err(e) => return Err(e),
                };
                return Ok(Authenticated {
                    principal: resolved.principal,
                    csrf_binding: None,
                });
            }
            let name = self.identity.config().cookie_name();
            Ok(cookie(&request.headers, name)
                .and_then(|value| self.identity.session(value))
                .map_or_else(Authenticated::anonymous, |resolved| {
                    Authenticated {
                        principal: resolved.principal,
                        csrf_binding: resolved.csrf_binding,
                    }
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
        let client = SafeClient::new(policy).map_err(|e| {
            Error::new(ErrorKind::Internal, format!("HTTP client: {e}"))
        })?;
        Ok(Self { client })
    }
}

impl ClientMetadataFetcher for GuardedFetcher {
    fn fetch<'a>(
        &'a self,
        url: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, Error>> {
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

fn invalid(message: String) -> Error {
    Error::new(ErrorKind::InvalidInput, message)
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// The single sign-on settings from the environment, or `None` when
/// `VELLUM_OIDC_ISSUER` is unset.
///
/// - `VELLUM_OIDC_ISSUER`: the provider's issuer URL (`https`, or `http`
///   on this machine for testing).
/// - `VELLUM_OIDC_CLIENT_ID` and, for a confidential client,
///   `VELLUM_OIDC_CLIENT_SECRET`.
/// - `VELLUM_OIDC_NAME`: the button label, "Sign in with {name}".
/// - `VELLUM_OIDC_ALLOWED_DOMAINS`: comma-separated email domains whose
///   people get an account on first sign-in.
/// - `VELLUM_OIDC_AUTO_CREATE`: `true` or `false`; defaults to `true`
///   when domains are listed, else `false` (existing accounts only).
///
/// # Errors
///
/// A malformed URL, a missing client ID or a bad flag.
pub fn oidc_config(
    var: impl Fn(&str) -> Option<String>,
) -> Result<Option<SsoConfig>, Error> {
    let get = |k: &str| {
        var(k)
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let Some(issuer) = get("VELLUM_OIDC_ISSUER") else {
        return Ok(None);
    };
    let issuer = Url::parse(&issuer).map_err(|e| {
        invalid(format!("VELLUM_OIDC_ISSUER is not a URL: {e}"))
    })?;
    if issuer.scheme() != "https"
        && !(issuer.scheme() == "http" && is_loopback(&issuer))
    {
        return Err(invalid(
            "VELLUM_OIDC_ISSUER must use https (http only on this machine)"
                .into(),
        ));
    }
    let client_id = get("VELLUM_OIDC_CLIENT_ID").ok_or_else(|| {
        invalid(
            "VELLUM_OIDC_ISSUER is set but VELLUM_OIDC_CLIENT_ID is not".into(),
        )
    })?;
    let allowed_domains: Vec<String> = get("VELLUM_OIDC_ALLOWED_DOMAINS")
        .map(|v| {
            v.split(',')
                .map(|d| d.trim().trim_start_matches('@').to_lowercase())
                .filter(|d| !d.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let auto_create = match get("VELLUM_OIDC_AUTO_CREATE").as_deref() {
        None => !allowed_domains.is_empty(),
        Some("1" | "true" | "yes") => true,
        Some("0" | "false" | "no") => false,
        Some(other) => {
            return Err(invalid(format!(
                "VELLUM_OIDC_AUTO_CREATE must be true or false, not {other:?}"
            )));
        }
    };
    let display_name = get("VELLUM_OIDC_NAME")
        .or_else(|| issuer.host_str().map(str::to_owned))
        .unwrap_or_else(|| "single sign-on".to_owned());
    Ok(Some(SsoConfig {
        issuer,
        client_id,
        client_secret: get("VELLUM_OIDC_CLIENT_SECRET"),
        display_name,
        allowed_domains,
        auto_create,
    }))
}

/// The largest provider response read (discovery, keys, tokens).
const MAX_PROVIDER_RESPONSE: u64 = 1024 * 1024;

/// Calls the identity provider through the SSRF guard. The issuer's own
/// host is trusted, so a provider on the local network works; other hosts
/// (Google's token endpoint, say) must be public `https` addresses.
#[derive(Debug, Clone)]
pub struct GuardedSsoHttp {
    client: SafeClient,
}

impl GuardedSsoHttp {
    /// Builds the client for `issuer`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the TLS backend cannot start.
    pub fn new(issuer: &Url) -> Result<Self, Error> {
        let mut policy = UrlPolicy::default();
        policy.allow_http = issuer.scheme() == "http";
        if let Some(port) = issuer.port_or_known_default()
            && !policy.allowed_ports.contains(&port)
        {
            policy.allowed_ports.push(port);
        }
        policy.trusted_hosts = issuer
            .host_str()
            .map(|h| h.trim_start_matches('[').trim_end_matches(']').to_owned())
            .into_iter()
            .collect();
        policy.max_redirects = 3;
        policy.timeout = Duration::from_secs(10);
        policy.max_body_bytes = MAX_PROVIDER_RESPONSE;
        let client = SafeClient::new(policy).map_err(|e| {
            Error::new(ErrorKind::Internal, format!("HTTP client: {e}"))
        })?;
        Ok(Self { client })
    }
}

fn provider_error(e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Unavailable, e.to_string())
}

impl SsoHttp for GuardedSsoHttp {
    fn get<'a>(
        &'a self,
        url: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, Error>> {
        Box::pin(async move {
            self.client
                .get_trusted_bytes(url)
                .await
                .map_err(provider_error)
        })
    }

    fn post_form<'a>(
        &'a self,
        url: &'a str,
        form: Vec<(String, String)>,
        basic: Option<(String, String)>,
    ) -> BoxFuture<'a, Result<(u16, Vec<u8>), Error>> {
        Box::pin(async move {
            let basic = basic.as_ref().map(|(u, p)| (u.as_str(), p.as_str()));
            self.client
                .post_form_trusted(url, &form, basic)
                .await
                .map_err(provider_error)
        })
    }
}

#[cfg(test)]
mod oidc_tests {
    use std::collections::HashMap;

    use super::*;

    fn read(pairs: &[(&str, &str)]) -> Result<Option<SsoConfig>, Error> {
        let vars: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        oidc_config(|k| vars.get(k).cloned())
    }

    #[test]
    fn only_scim_paths_skip_the_api_token_check() {
        assert!(is_scim("/scim/v2"));
        assert!(is_scim("/scim/v2/Users"));
        assert!(!is_scim("/scim/v2x"));
        assert!(!is_scim("/api/v1/scim/v2"));
    }

    #[test]
    fn single_sign_on_is_off_without_an_issuer() {
        assert!(read(&[]).unwrap().is_none());
    }

    #[test]
    fn domains_turn_on_account_creation_by_default() {
        let config = read(&[
            ("VELLUM_OIDC_ISSUER", "https://id.northstar.dev"),
            ("VELLUM_OIDC_CLIENT_ID", "vellum"),
            (
                "VELLUM_OIDC_ALLOWED_DOMAINS",
                "northstar.dev, @Labs.Northstar.dev",
            ),
        ])
        .unwrap()
        .unwrap();
        assert!(config.auto_create);
        assert_eq!(
            config.allowed_domains,
            ["northstar.dev", "labs.northstar.dev"]
        );
        assert_eq!(config.display_name, "id.northstar.dev");
        assert!(config.client_secret.is_none());
        let off = read(&[
            ("VELLUM_OIDC_ISSUER", "https://id.northstar.dev"),
            ("VELLUM_OIDC_CLIENT_ID", "vellum"),
        ])
        .unwrap()
        .unwrap();
        assert!(!off.auto_create);
    }

    #[test]
    fn bad_settings_stop_the_server() {
        for bad in [
            vec![("VELLUM_OIDC_ISSUER", "https://id.northstar.dev")],
            vec![
                ("VELLUM_OIDC_ISSUER", "http://id.northstar.dev"),
                ("VELLUM_OIDC_CLIENT_ID", "v"),
            ],
            vec![
                ("VELLUM_OIDC_ISSUER", "https://id.northstar.dev"),
                ("VELLUM_OIDC_CLIENT_ID", "v"),
                ("VELLUM_OIDC_AUTO_CREATE", "maybe"),
            ],
        ] {
            assert!(read(&bad).is_err(), "{bad:?}");
        }
        assert!(
            read(&[
                (
                    "VELLUM_OIDC_ISSUER",
                    "http://localhost:8080/realms/northstar"
                ),
                ("VELLUM_OIDC_CLIENT_ID", "v"),
            ])
            .is_ok()
        );
    }
}
