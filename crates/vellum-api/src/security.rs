// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Security headers and CSRF protection.
//!
//! Every response carries a strict Content Security Policy, HSTS (when
//! served over HTTPS), `nosniff`, a no-referrer-on-downgrade policy, framing
//! protection and cross-origin isolation headers. Public pages get a
//! stricter, sandboxed policy because they show user content on a separate
//! origin.
//!
//! CSRF defence has three layers for unsafe methods (`POST`, `PUT`,
//! `PATCH`, `DELETE`): browsers that send `Sec-Fetch-Site: cross-site` are
//! refused; an `Origin` header must be the app's own; and requests
//! authenticated by the session cookie must echo the session's CSRF token in
//! `X-CSRF-Token`. Bearer-token requests carry no ambient authority and skip
//! the token check.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit as _, Mac as _};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use sha2::Sha256;
use subtle::ConstantTimeEq as _;
use vellum_kernel::{Error, ErrorKind};

use crate::config::GatewayConfig;

/// The header carrying the CSRF token.
pub const CSRF_HEADER: HeaderName = HeaderName::from_static("x-csrf-token");

/// Which policy a response gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// The web app and its API.
    App,
    /// Public pages and share links, on their own origin.
    Public,
}

/// The app's own origin with a WebSocket scheme (`wss://` for `https://`),
/// for the live-editing socket at `/collab`. CSP Level 3 lets `'self'`
/// match it, but older engines need it named.
fn socket_origin(app_origin: &str) -> Option<String> {
    let (scheme, rest) = app_origin.split_once("://")?;
    let scheme = match scheme.to_ascii_lowercase().as_str() {
        "https" => "wss",
        "http" => "ws",
        _ => return None,
    };
    let host = rest.split('/').next().filter(|h| !h.is_empty())?;
    Some(format!("{scheme}://{host}"))
}

/// The app's Content Security Policy.
#[must_use]
pub fn app_csp(config: &GatewayConfig) -> String {
    let frames = if config.embed_origins.is_empty() {
        "'none'".to_owned()
    } else {
        config.embed_origins.join(" ")
    };
    let connect = socket_origin(&config.app_origin)
        .map_or_else(|| "'self'".to_owned(), |ws| format!("'self' {ws}"));
    let mut csp = format!(
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src \
         'self' data: blob:; font-src 'self'; connect-src {connect}; media-src \
         'self' blob:; worker-src 'self'; manifest-src 'self'; frame-src \
         {frames}; frame-ancestors 'none'; base-uri 'none'; form-action \
         'self'; object-src 'none'"
    );
    if config.https {
        csp.push_str("; upgrade-insecure-requests");
    }
    csp
}

/// The public origin's policy: no scripts at all, and sandboxed.
#[must_use]
pub const fn public_csp() -> &'static str {
    "default-src 'none'; style-src 'self'; img-src 'self' data:; font-src \
     'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; \
     object-src 'none'; sandbox allow-forms allow-same-origin allow-popups \
     allow-popups-to-escape-sandbox"
}

fn set(headers: &mut HeaderMap, name: HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        let _ = headers.insert(name, value);
    }
}

/// Adds the security headers to a response. Headers a handler already set
/// (a download's own CSP, say) are kept.
pub fn apply_headers(
    headers: &mut HeaderMap,
    surface: Surface,
    config: &GatewayConfig,
) {
    let defaults: [(HeaderName, String); 8] = [
        (
            http::header::CONTENT_SECURITY_POLICY,
            match surface {
                Surface::App => app_csp(config),
                Surface::Public => public_csp().to_owned(),
            },
        ),
        (http::header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
        (
            http::header::REFERRER_POLICY,
            "strict-origin-when-cross-origin".into(),
        ),
        (http::header::X_FRAME_OPTIONS, "DENY".into()),
        (
            HeaderName::from_static("permissions-policy"),
            "camera=(), microphone=(), geolocation=(), payment=(), usb=(), \
                interest-cohort=()"
                .into(),
        ),
        (
            HeaderName::from_static("cross-origin-opener-policy"),
            "same-origin".into(),
        ),
        (
            HeaderName::from_static("cross-origin-resource-policy"),
            match surface {
                Surface::App => "same-origin".into(),
                Surface::Public => "same-site".into(),
            },
        ),
        (
            HeaderName::from_static("x-permitted-cross-domain-policies"),
            "none".into(),
        ),
    ];
    for (name, value) in defaults {
        if !headers.contains_key(&name) {
            set(headers, name, &value);
        }
    }
    if config.https {
        set(
            headers,
            http::header::STRICT_TRANSPORT_SECURITY,
            "max-age=63072000; includeSubDomains",
        );
    }
}

/// Derives the CSRF token for a session binding.
#[must_use]
pub fn csrf_token(key: &[u8], binding: &[u8; 32]) -> String {
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(key) else {
        return String::new();
    };
    mac.update(b"vellum-csrf-v1");
    mac.update(binding);
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

/// Whether the method changes state.
#[must_use]
#[expect(
    clippy::missing_const_for_fn,
    reason = "Method comparison is not const"
)]
pub fn is_unsafe(method: &Method) -> bool {
    !matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

/// Checks an unsafe request for cross-site forgery.
///
/// # Errors
///
/// `permission_denied` when the request comes from another site or lacks a
/// valid CSRF token.
pub fn check_csrf(
    method: &Method,
    headers: &HeaderMap,
    binding: Option<&[u8; 32]>,
    config: &GatewayConfig,
) -> Result<(), Error> {
    if !is_unsafe(method) {
        return Ok(());
    }
    let deny =
        |why: &'static str| Err(Error::new(ErrorKind::PermissionDenied, why));
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if header("sec-fetch-site") == Some("cross-site") {
        return deny("cross-site requests are not allowed");
    }
    if header("origin")
        .is_some_and(|origin| !origin.eq_ignore_ascii_case(&config.app_origin))
    {
        return deny("cross-origin requests are not allowed");
    }
    if let Some(binding) = binding {
        let expected = csrf_token(&config.csrf_key, binding);
        let given = header(CSRF_HEADER.as_str()).unwrap_or_default();
        if expected.is_empty()
            || !bool::from(expected.as_bytes().ct_eq(given.as_bytes()))
        {
            return deny("missing or invalid CSRF token; reload the page");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> GatewayConfig {
        GatewayConfig::new("https://vellum.northstar.dev", vec![7; 32])
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            if let Ok(v) = HeaderValue::from_str(v) {
                let _ = h.insert(HeaderName::from_static(k), v);
            }
        }
        h
    }

    #[test]
    fn csrf_rules() {
        let c = config();
        let b = [3u8; 32];
        let token = csrf_token(&c.csrf_key, &b);
        assert_eq!(token.len(), 43);
        let post = Method::POST;
        assert!(
            check_csrf(
                &Method::GET,
                &headers(&[("sec-fetch-site", "cross-site")]),
                Some(&b),
                &c
            )
            .is_ok()
        );
        assert!(
            check_csrf(
                &post,
                &headers(&[("sec-fetch-site", "cross-site")]),
                None,
                &c
            )
            .is_err()
        );
        assert!(
            check_csrf(
                &post,
                &headers(&[("origin", "https://evil.example")]),
                None,
                &c
            )
            .is_err()
        );
        assert!(
            check_csrf(
                &post,
                &headers(&[("origin", "https://vellum.northstar.dev")]),
                None,
                &c
            )
            .is_ok()
        );
        assert!(check_csrf(&post, &headers(&[]), Some(&b), &c).is_err());
        assert!(
            check_csrf(
                &post,
                &headers(&[("x-csrf-token", "nope")]),
                Some(&b),
                &c
            )
            .is_err()
        );
        assert!(
            check_csrf(
                &post,
                &headers(&[("x-csrf-token", &token)]),
                Some(&b),
                &c
            )
            .is_ok()
        );
        assert_ne!(csrf_token(&c.csrf_key, &[4; 32]), token);
    }

    #[test]
    fn headers_are_strict() {
        let mut c = config();
        let mut h = HeaderMap::new();
        apply_headers(&mut h, Surface::App, &c);
        let csp = h
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(csp.contains("script-src 'self'"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert!(!csp.contains("unsafe"));
        assert!(h.contains_key("strict-transport-security"));
        let mut p = HeaderMap::new();
        apply_headers(&mut p, Surface::Public, &c);
        let csp = p
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(csp.contains("sandbox"));
        assert!(!csp.contains("script-src"));
        c.https = false;
        c.embed_origins = vec!["https://www.youtube-nocookie.com".into()];
        let mut h = HeaderMap::new();
        apply_headers(&mut h, Surface::App, &c);
        assert!(!h.contains_key("strict-transport-security"));
        let csp = h
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(csp.contains("frame-src https://www.youtube-nocookie.com"));
    }

    #[test]
    fn the_app_may_open_its_own_socket() {
        let csp = app_csp(&config());
        assert!(csp.contains("connect-src 'self' wss://vellum.northstar.dev;"));
        let plain = GatewayConfig::new("http://localhost:8080", vec![7; 32]);
        assert!(
            app_csp(&plain).contains("connect-src 'self' ws://localhost:8080;")
        );
        assert_eq!(socket_origin("localhost"), None);
    }
}
