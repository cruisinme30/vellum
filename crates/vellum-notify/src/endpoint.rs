// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Outbound endpoints: the SSRF policy for webhooks and push services, and
//! the HTTP transport that enforces it.
//!
//! The address rules are the shared ones in [`vellum_security::ssrf`]. A
//! URL is checked when an admin saves the hook, and the transport resolves
//! names through [`GuardedResolver`], which refuses any non-public address
//! at connect time, so DNS rebinding cannot reach the internal network.
//! Webhook requests never follow redirects: a 3xx is a failed delivery.

use std::sync::Arc;
use std::time::Duration;

use url::Url;
use vellum_security::ssrf::{GuardedResolver, SsrfError, UrlPolicy};

use crate::delivery::{HttpResponse, HttpTransport, TransportError};
use crate::webhook::OutboundRequest;

/// The per-attempt budget for a delivery.
pub const TIMEOUT: Duration = Duration::from_secs(10);

/// The policy for webhook and push endpoints: HTTPS only on 443 or 8443.
///
/// `trusted_hosts` exempts named hosts from the address rules. It is empty
/// in the Team and Org profiles; on a laptop an admin may name a local
/// receiver (`localhost`) explicitly, and only then may `http` be used.
#[must_use]
pub fn policy(trusted_hosts: &[String]) -> UrlPolicy {
    let mut policy = UrlPolicy::default();
    policy.allow_http = !trusted_hosts.is_empty();
    policy.allowed_ports = vec![443, 8443];
    policy.trusted_hosts = trusted_hosts
        .iter()
        .map(|h| h.to_ascii_lowercase())
        .collect();
    policy.max_redirects = 0;
    policy.timeout = TIMEOUT;
    policy
}

/// Checks a URL when a hook is saved.
///
/// # Errors
///
/// [`SsrfError`] names the first rule the URL breaks.
pub fn check_url(raw: &str, policy: &UrlPolicy) -> Result<Url, SsrfError> {
    let url = Url::parse(raw)?;
    let trusted = url.host_str().is_some_and(|h| {
        policy
            .trusted_hosts
            .iter()
            .any(|t| t.eq_ignore_ascii_case(h))
    });
    if trusted {
        // An admin named this receiver in the policy, so it is checked as
        // a trusted host: the address and port rules do not apply to it.
        policy.check_trusted_url(&url)?;
    } else {
        policy.check_url(&url)?;
    }
    Ok(url)
}

/// An [`HttpTransport`] over `reqwest` with the guarded resolver.
#[derive(Debug, Clone)]
pub struct SafeTransport {
    client: reqwest::Client,
    policy: Arc<UrlPolicy>,
}

impl SafeTransport {
    /// Builds the transport.
    ///
    /// # Errors
    ///
    /// [`SsrfError::Http`] if the TLS backend cannot initialise.
    pub fn new(policy: UrlPolicy) -> Result<Self, SsrfError> {
        let policy = Arc::new(policy);
        let client = reqwest::Client::builder()
            .no_proxy()
            .dns_resolver(GuardedResolver::new(Arc::clone(&policy)))
            .timeout(policy.timeout)
            .connect_timeout(policy.timeout)
            .https_only(!policy.allow_http)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(crate::webhook::USER_AGENT)
            .build()?;
        Ok(Self { client, policy })
    }
}

fn refused(error: &reqwest::Error) -> Option<String> {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = source {
        if let Some(ssrf) = current.downcast_ref::<SsrfError>() {
            return Some(ssrf.to_string());
        }
        source = current.source();
    }
    None
}

impl HttpTransport for SafeTransport {
    async fn post(&self, request: &OutboundRequest) -> Result<HttpResponse, TransportError> {
        let _ = check_url(request.url.as_str(), &self.policy)
            .map_err(|e| TransportError::Refused(e.to_string()))?;
        let mut builder = self
            .client
            .post(request.url.clone())
            .body(request.body.clone());
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        match builder.send().await {
            Ok(response) => Ok(HttpResponse {
                status: response.status().as_u16(),
                retry_after: response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse().ok()),
            }),
            Err(error) => Err(refused(&error).map_or_else(
                || {
                    if error.is_timeout() {
                        TransportError::Timeout
                    } else {
                        TransportError::Connect(error.to_string())
                    }
                },
                TransportError::Refused,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_policy() {
        let strict = policy(&[]);
        assert!(check_url("https://hooks.northstar.dev/vellum", &strict).is_ok());
        assert!(check_url("https://hooks.northstar.dev:8443/v", &strict).is_ok());
        for bad in [
            "http://hooks.northstar.dev",
            "https://hooks.northstar.dev:22/",
            "https://user:pw@northstar.dev",
            "https://localhost/x",
            "https://10.0.0.1/",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/",
            "ftp://northstar.dev",
        ] {
            assert!(check_url(bad, &strict).is_err(), "{bad}");
        }
    }

    #[test]
    fn trusted_local_receivers() {
        let laptop = policy(&["localhost".to_owned()]);
        assert!(check_url("http://localhost:8080/hook", &laptop).is_ok());
        assert!(check_url("http://127.0.0.1:8080/hook", &laptop).is_err());
    }
}
