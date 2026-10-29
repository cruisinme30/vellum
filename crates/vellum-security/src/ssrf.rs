// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Server-side request forgery (SSRF) protection.
//!
//! Every outbound fetch made on a user's behalf (link previews, `embed`
//! and `link` containers, image imports, webhooks) goes through
//! [`SafeClient`]. It refuses, before connecting:
//!
//! - schemes other than `http`/`https`, URLs with credentials, and ports
//!   outside the allow-list;
//! - IP-literal hosts and resolved addresses that are not public unicast
//!   (loopback, private, link-local and cloud metadata, CGNAT, multicast,
//!   documentation, `IPv4` embedded in `IPv6`, and so on);
//! - redirects to any URL that fails the same checks.
//!
//! DNS is resolved once by [`GuardedResolver`], and the connection uses
//! exactly the addresses that were checked, so DNS rebinding cannot swap in
//! an internal address. Environment proxies are ignored for the same reason.
//! Trusted internal services (the Kroki sidecar) are reached by naming
//! them in [`UrlPolicy::trusted_hosts`], never by relaxing the address rules.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use tracing::Instrument;
use url::{Host, Url};

/// Why an outbound request was refused or failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SsrfError {
    /// The text is not a valid absolute URL.
    #[error("invalid URL: {0}")]
    InvalidUrl(#[from] url::ParseError),
    /// The scheme is not `http` or `https` (or `http` is disabled).
    #[error("scheme {0:?} is not allowed")]
    Scheme(String),
    /// The URL carries a user name or password.
    #[error("URLs with credentials are not allowed")]
    Credentials,
    /// The port is not in the allow-list.
    #[error("port {0} is not allowed")]
    Port(u16),
    /// The host name is reserved for internal use.
    #[error("host {0:?} is not allowed")]
    Host(String),
    /// The host is, or resolved to, a non-public address.
    #[error("address {0} is not public")]
    Address(IpAddr),
    /// The host did not resolve to any address.
    #[error("host {0:?} did not resolve")]
    Unresolved(String),
    /// Too many redirects.
    #[error("more than {0} redirects")]
    Redirects(usize),
    /// The response body exceeded the size limit.
    #[error("response larger than {0} bytes")]
    TooLarge(u64),
    /// The HTTP request failed.
    #[error("request failed: {0}")]
    Http(#[from] reqwest::Error),
}

/// What outbound requests may reach.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UrlPolicy {
    /// Allow plain `http` as well as `https`.
    pub allow_http: bool,
    /// Ports a URL may name.
    pub allowed_ports: Vec<u16>,
    /// Host names exempt from the address rules (lowercase, exact match),
    /// such as `kroki` in the compose deployment. Empty by default.
    pub trusted_hosts: Vec<String>,
    /// Redirects followed before giving up.
    pub max_redirects: usize,
    /// Total time budget per request.
    pub timeout: Duration,
    /// Largest response body [`SafeClient::get_bytes`] will read.
    pub max_body_bytes: u64,
}

impl Default for UrlPolicy {
    fn default() -> Self {
        Self {
            allow_http: true,
            allowed_ports: vec![80, 443],
            trusted_hosts: Vec::new(),
            max_redirects: 5,
            timeout: Duration::from_secs(10),
            max_body_bytes: 10 * 1024 * 1024,
        }
    }
}

impl UrlPolicy {
    fn is_trusted(&self, host: &str) -> bool {
        self.trusted_hosts
            .iter()
            .any(|t| t.eq_ignore_ascii_case(host))
    }

    /// Checks a URL's scheme, credentials, port and host before any I/O.
    ///
    /// # Errors
    ///
    /// Returns the first rule the URL breaks.
    pub fn check_url(&self, url: &Url) -> Result<(), SsrfError> {
        match url.scheme() {
            "https" => {}
            "http" if self.allow_http => {}
            other => return Err(SsrfError::Scheme(other.to_owned())),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(SsrfError::Credentials);
        }
        let port = url.port_or_known_default().unwrap_or(0);
        match url.host() {
            Some(Host::Domain(d)) if self.is_trusted(d) => return Ok(()),
            Some(Host::Domain(d)) => check_domain(d)?,
            Some(Host::Ipv4(ip)) => check_ip(IpAddr::V4(ip))?,
            Some(Host::Ipv6(ip)) => check_ip(IpAddr::V6(ip))?,
            None => return Err(SsrfError::Host(String::new())),
        }
        if self.allowed_ports.contains(&port) {
            Ok(())
        } else {
            Err(SsrfError::Port(port))
        }
    }
}

/// Rejects names that only make sense inside a network.
fn check_domain(domain: &str) -> Result<(), SsrfError> {
    let d = domain.trim_end_matches('.').to_ascii_lowercase();
    let internal = d == "localhost"
        || !d.contains('.')
        || [
            ".localhost",
            ".local",
            ".internal",
            ".home.arpa",
            ".lan",
            ".corp",
        ]
        .iter()
        .any(|suffix| d.ends_with(suffix));
    if internal {
        Err(SsrfError::Host(domain.to_owned()))
    } else {
        Ok(())
    }
}

fn check_ip(ip: IpAddr) -> Result<(), SsrfError> {
    if is_public(ip) {
        Ok(())
    } else {
        Err(SsrfError::Address(ip))
    }
}

/// Whether `ip` is a public unicast address that may be fetched.
#[must_use]
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0                                   // "this network"
        || a == 10                             // private
        || (a == 100 && (64..128).contains(&b)) // CGNAT
        || a == 127                            // loopback
        || (a == 169 && b == 254)              // link-local, cloud metadata
        || (a == 172 && (16..32).contains(&b)) // private
        || (a == 192 && b == 0 && (c == 0 || c == 2)) // IETF, TEST-NET-1
        || (a == 192 && b == 88 && c == 99)    // 6to4 relay
        || (a == 192 && b == 168)              // private
        || (a == 198 && (b == 18 || b == 19))  // benchmarking
        || (a == 198 && b == 51 && c == 100)   // TEST-NET-2
        || (a == 203 && b == 0 && c == 113)    // TEST-NET-3
        || a >= 224) // multicast, reserved, broadcast
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let [s0, s1, s2, s3, s4, s5, s6, s7] = ip.segments();
    // NAT64 (64:ff9b::/96) and 6to4 (2002::/16) embed an IPv4 address.
    if s0 == 0x64 && s1 == 0xff9b && s2 == 0 && s3 == 0 && s4 == 0 && s5 == 0 {
        return is_public_v4(v4_from(s6, s7));
    }
    if s0 == 0x2002 {
        return is_public_v4(v4_from(s1, s2));
    }
    // Only global unicast (2000::/3), minus Teredo and documentation.
    s0 & 0xe000 == 0x2000 && !(s0 == 0x2001 && (s1 == 0 || s1 == 0xdb8))
}

const fn v4_from(hi: u16, lo: u16) -> Ipv4Addr {
    let [a, b] = hi.to_be_bytes();
    let [c, d] = lo.to_be_bytes();
    Ipv4Addr::new(a, b, c, d)
}

/// A DNS resolver that fails when any address for a name is not public.
///
/// Rejecting the whole answer, rather than filtering it, keeps a name that
/// mixes public and internal records from being used to probe either.
#[derive(Debug, Clone)]
pub struct GuardedResolver {
    policy: Arc<UrlPolicy>,
}

impl GuardedResolver {
    /// Creates a resolver enforcing `policy`.
    #[must_use]
    pub const fn new(policy: Arc<UrlPolicy>) -> Self {
        Self { policy }
    }

    /// Resolves `host` and checks every address.
    ///
    /// # Errors
    ///
    /// Returns [`SsrfError::Unresolved`] or [`SsrfError::Address`].
    pub async fn resolve_checked(&self, host: &str) -> Result<Vec<SocketAddr>, SsrfError> {
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, 0))
            .await
            .map_err(|_| SsrfError::Unresolved(host.to_owned()))?
            .collect();
        if addrs.is_empty() {
            return Err(SsrfError::Unresolved(host.to_owned()));
        }
        if !self.policy.is_trusted(host)
            && let Some(bad) = addrs.iter().find(|a| !is_public(a.ip()))
        {
            tracing::warn!(host, address = %bad.ip(), "ssrf.blocked");
            return Err(SsrfError::Address(bad.ip()));
        }
        Ok(addrs)
    }
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let this = self.clone();
        Box::pin(async move {
            let addrs = this.resolve_checked(name.as_str()).await?;
            let addrs: Addrs = Box::new(addrs.into_iter());
            Ok(addrs)
        })
    }
}

/// An HTTP client that can only reach what its [`UrlPolicy`] allows.
#[derive(Debug, Clone)]
pub struct SafeClient {
    client: reqwest::Client,
    policy: Arc<UrlPolicy>,
}

impl SafeClient {
    /// Builds a client: guarded DNS, checked redirects, no proxies, timeouts.
    ///
    /// # Errors
    ///
    /// Returns [`SsrfError::Http`] if the TLS backend cannot initialise.
    pub fn new(policy: UrlPolicy) -> Result<Self, SsrfError> {
        let policy = Arc::new(policy);
        let redirect_policy = Arc::clone(&policy);
        let client = reqwest::Client::builder()
            .no_proxy()
            .dns_resolver(GuardedResolver::new(Arc::clone(&policy)))
            .timeout(policy.timeout)
            .connect_timeout(policy.timeout)
            .https_only(!policy.allow_http)
            .redirect(reqwest::redirect::Policy::custom(move |attempt| {
                if attempt.previous().len() > redirect_policy.max_redirects {
                    let err = SsrfError::Redirects(redirect_policy.max_redirects);
                    return attempt.error(err);
                }
                match redirect_policy.check_url(attempt.url()) {
                    Ok(()) => attempt.follow(),
                    Err(err) => attempt.error(err),
                }
            }))
            .build()?;
        Ok(Self { client, policy })
    }

    /// Sends a GET after checking the URL.
    ///
    /// # Errors
    ///
    /// Returns an [`SsrfError`] if the URL, any address or any redirect is
    /// refused, or the request fails.
    pub async fn get(&self, url: &str) -> Result<reqwest::Response, SsrfError> {
        let url = Url::parse(url)?;
        self.policy.check_url(&url)?;
        let span = tracing::info_span!("ssrf.fetch", host = url.host_str().unwrap_or_default());
        let response = self.client.get(url).send().instrument(span).await?;
        Ok(response.error_for_status()?)
    }

    /// Sends a GET and reads at most [`UrlPolicy::max_body_bytes`].
    ///
    /// # Errors
    ///
    /// As [`SafeClient::get`], plus [`SsrfError::TooLarge`].
    pub async fn get_bytes(&self, url: &str) -> Result<Vec<u8>, SsrfError> {
        let limit = self.policy.max_body_bytes;
        let mut response = self.get(url).await?;
        if response.content_length().is_some_and(|len| len > limit) {
            return Err(SsrfError::TooLarge(limit));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if body.len() as u64 + chunk.len() as u64 > limit {
                return Err(SsrfError::TooLarge(limit));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}
