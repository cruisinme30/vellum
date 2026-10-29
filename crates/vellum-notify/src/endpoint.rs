// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Outbound endpoint checks: the SSRF guard for webhooks and push services.
//!
//! An admin-supplied URL must never let Vellum reach its own network. The
//! check runs twice: [`check_url`] when the hook is saved, and
//! [`check_resolved`] on the addresses the transport resolved, right before
//! it connects, so DNS rebinding cannot slip a private address in between.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::{Host, Url};

/// Whether plain `http://` and private addresses are allowed.
///
/// `Strict` is the default and the only mode for the Team and Org
/// profiles. `AllowPrivate` exists for the single-binary laptop profile,
/// where the receiver often runs on the same machine; an admin must turn it
/// on explicitly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Policy {
    /// HTTPS only, public addresses only.
    #[default]
    Strict,
    /// HTTP and private or loopback addresses are allowed.
    AllowPrivate,
}

/// Why an endpoint was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EndpointError {
    /// Not a valid absolute URL.
    #[error("not a valid URL")]
    Invalid,
    /// Only `https` (or `http` when allowed) is accepted.
    #[error("scheme `{0}` is not allowed")]
    Scheme(String),
    /// Credentials in the URL would leak into logs.
    #[error("URLs must not carry a user name or password")]
    Credentials,
    /// The URL has no host.
    #[error("URL has no host")]
    NoHost,
    /// The host is or resolved to a non-public address.
    #[error("address {0} is not public")]
    PrivateAddress(IpAddr),
    /// The host name is reserved for local use.
    #[error("host `{0}` is reserved for local use")]
    LocalName(String),
    /// Resolution returned no addresses.
    #[error("host did not resolve")]
    Unresolved,
}

/// Checks a URL when it is saved.
///
/// # Errors
///
/// [`EndpointError`] names the first rule the URL breaks.
pub fn check_url(raw: &str, policy: Policy) -> Result<Url, EndpointError> {
    let url = Url::parse(raw).map_err(|_| EndpointError::Invalid)?;
    match (url.scheme(), policy) {
        ("https", _) | ("http", Policy::AllowPrivate) => {}
        (other, _) => return Err(EndpointError::Scheme(other.to_owned())),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(EndpointError::Credentials);
    }
    if policy == Policy::AllowPrivate {
        return url.host().map(|_| url.clone()).ok_or(EndpointError::NoHost);
    }
    match url.host() {
        None => Err(EndpointError::NoHost),
        Some(Host::Ipv4(ip)) => check_ip(IpAddr::V4(ip)).map(|()| url.clone()),
        Some(Host::Ipv6(ip)) => check_ip(IpAddr::V6(ip)).map(|()| url.clone()),
        Some(Host::Domain(name)) => {
            let lower = name.to_ascii_lowercase();
            let local = ["localhost", "local", "internal", "localdomain", "home.arpa"]
                .iter()
                .any(|suffix| lower == *suffix || lower.ends_with(&format!(".{suffix}")));
            if local || !lower.contains('.') {
                Err(EndpointError::LocalName(lower))
            } else {
                Ok(url.clone())
            }
        }
    }
}

/// Checks every resolved address right before connecting.
///
/// # Errors
///
/// [`EndpointError::PrivateAddress`] if any address is not public, and
/// [`EndpointError::Unresolved`] if there are none.
pub fn check_resolved(addrs: &[IpAddr], policy: Policy) -> Result<(), EndpointError> {
    if addrs.is_empty() {
        return Err(EndpointError::Unresolved);
    }
    if policy == Policy::AllowPrivate {
        return Ok(());
    }
    addrs.iter().try_for_each(|ip| check_ip(*ip))
}

fn check_ip(ip: IpAddr) -> Result<(), EndpointError> {
    if is_public(ip) {
        Ok(())
    } else {
        Err(EndpointError::PrivateAddress(ip))
    }
}

/// Whether an address is globally routable.
#[must_use]
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => v6
            .to_ipv4_mapped()
            .map_or_else(|| is_public_v6(v6), is_public_v4),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_multicast()
        || a == 0
        // 100.64.0.0/10 carrier-grade NAT
        || (a == 100 && (64..128).contains(&b))
        // 192.0.0.0/24 IETF protocol assignments
        || (a == 192 && b == 0 && c == 0)
        // 198.18.0.0/15 benchmarking
        || (a == 198 && (b == 18 || b == 19))
        // 240.0.0.0/4 reserved
        || a >= 240)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments().first().copied().unwrap_or(0);
    !(ip.is_unspecified()
        || ip.is_loopback()
        || ip.is_multicast()
        // fc00::/7 unique local
        || (first & 0xfe00) == 0xfc00
        // fe80::/10 link local
        || (first & 0xffc0) == 0xfe80
        // 2001:db8::/32 documentation
        || (first == 0x2001 && ip.segments().get(1) == Some(&0x0db8))
        // 64:ff9b::/96 NAT64 can reach IPv4 private ranges
        || first == 0x0064)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_accepts_public_https() {
        assert!(check_url("https://hooks.northstar.dev/vellum", Policy::Strict).is_ok());
        assert!(check_url("https://8.8.8.8/x", Policy::Strict).is_ok());
    }

    #[test]
    fn strict_refuses_local_targets() {
        let cases = [
            (
                "http://hooks.northstar.dev",
                EndpointError::Scheme("http".to_owned()),
            ),
            ("https://user:pw@northstar.dev", EndpointError::Credentials),
            (
                "https://localhost/x",
                EndpointError::LocalName("localhost".to_owned()),
            ),
            (
                "https://db.internal/x",
                EndpointError::LocalName("db.internal".to_owned()),
            ),
            (
                "https://intranet/x",
                EndpointError::LocalName("intranet".to_owned()),
            ),
            (
                "ftp://northstar.dev",
                EndpointError::Scheme("ftp".to_owned()),
            ),
        ];
        for (url, expected) in cases {
            assert_eq!(check_url(url, Policy::Strict), Err(expected), "{url}");
        }
        for url in [
            "https://127.0.0.1/",
            "https://10.1.2.3/",
            "https://169.254.169.254/latest/meta-data",
            "https://[::1]/",
            "https://[fd00::1]/",
            "https://[::ffff:10.0.0.1]/",
            "https://100.64.0.1/",
        ] {
            assert!(
                matches!(
                    check_url(url, Policy::Strict),
                    Err(EndpointError::PrivateAddress(_))
                ),
                "{url}"
            );
        }
    }

    #[test]
    fn allow_private_is_for_laptops() {
        assert!(check_url("http://localhost:8080/hook", Policy::AllowPrivate).is_ok());
        assert!(check_url("ftp://localhost", Policy::AllowPrivate).is_err());
    }

    #[test]
    fn resolved_addresses_are_rechecked() {
        let public: IpAddr = Ipv4Addr::new(93, 184, 216, 34).into();
        let private: IpAddr = Ipv4Addr::new(192, 168, 1, 1).into();
        assert_eq!(check_resolved(&[public], Policy::Strict), Ok(()));
        assert_eq!(
            check_resolved(&[public, private], Policy::Strict),
            Err(EndpointError::PrivateAddress(private))
        );
        assert_eq!(
            check_resolved(&[], Policy::Strict),
            Err(EndpointError::Unresolved)
        );
        assert_eq!(check_resolved(&[private], Policy::AllowPrivate), Ok(()));
    }
}
