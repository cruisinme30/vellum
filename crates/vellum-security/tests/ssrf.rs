// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! SSRF guard: address rules, URL rules and a live loopback server.

#![allow(clippy::unwrap_used)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use proptest::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;
use vellum_security::ssrf::{SafeClient, SsrfError, UrlPolicy, is_public};

#[test]
fn address_table() {
    let blocked = [
        "0.0.0.0",
        "10.1.2.3",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.169.254",
        "172.16.0.1",
        "172.31.255.255",
        "192.0.2.1",
        "192.168.1.1",
        "198.18.0.1",
        "203.0.113.9",
        "224.0.0.1",
        "255.255.255.255",
        "::",
        "::1",
        "fe80::1",
        "fc00::1",
        "fd12::1",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "64:ff9b::a00:1",
        "2002:a00:1::",
        "2001:db8::1",
        "2001::1",
        "ff02::1",
    ];
    for ip in blocked {
        assert!(!is_public(ip.parse().unwrap()), "{ip} should be blocked");
    }
    let allowed = [
        "1.1.1.1",
        "8.8.8.8",
        "172.32.0.1",
        "100.128.0.1",
        "2606:4700::1111",
        "::ffff:8.8.8.8",
        "64:ff9b::808:808",
    ];
    for ip in allowed {
        assert!(is_public(ip.parse().unwrap()), "{ip} should be allowed");
    }
}

proptest! {
    #[test]
    fn mapped_v6_matches_v4(bits in any::<u32>()) {
        let v4 = Ipv4Addr::from(bits);
        prop_assert_eq!(is_public(IpAddr::V6(v4.to_ipv6_mapped())), is_public(IpAddr::V4(v4)));
    }

    #[test]
    fn private_ranges_never_public(host in any::<u32>()) {
        let [_, b, c, d] = host.to_be_bytes();
        prop_assert!(!is_public(IpAddr::V4(Ipv4Addr::new(10, b, c, d))));
        prop_assert!(!is_public(IpAddr::V4(Ipv4Addr::new(127, b, c, d))));
        prop_assert!(!is_public(IpAddr::V6(Ipv6Addr::new(0xfe80, 0, 0, 0, u16::from(b), 0, 0, u16::from(d)))));
    }
}

fn check(url: &str) -> Result<(), SsrfError> {
    UrlPolicy::default().check_url(&Url::parse(url).unwrap())
}

#[test]
fn url_rules() {
    assert!(check("https://northstar.dev/roadmap").is_ok());
    assert!(matches!(
        check("file:///etc/passwd"),
        Err(SsrfError::Scheme(_))
    ));
    assert!(matches!(
        check("gopher://northstar.dev/"),
        Err(SsrfError::Scheme(_))
    ));
    assert!(matches!(
        check("https://user:pw@northstar.dev/"),
        Err(SsrfError::Credentials)
    ));
    assert!(matches!(
        check("https://northstar.dev:6379/"),
        Err(SsrfError::Port(6379))
    ));
    assert!(matches!(
        check("http://169.254.169.254/latest/meta-data"),
        Err(SsrfError::Address(_))
    ));
    assert!(matches!(check("http://[::1]/"), Err(SsrfError::Address(_))));
    assert!(matches!(
        check("http://2130706433/"),
        Err(SsrfError::Address(_))
    ));
    for host in [
        "localhost",
        "db",
        "kroki",
        "printer.local",
        "metadata.google.internal",
        "x.localhost.",
    ] {
        assert!(
            matches!(check(&format!("http://{host}/")), Err(SsrfError::Host(_))),
            "{host}"
        );
    }
    let mut no_http = UrlPolicy::default();
    no_http.allow_http = false;
    assert!(
        no_http
            .check_url(&Url::parse("http://northstar.dev/").unwrap())
            .is_err()
    );
}

/// Serves one canned HTTP response per connection on loopback.
async fn serve(response: String) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buf = [0_u8; 1024];
            let _ = socket.read(&mut buf).await;
            let _ = socket.write_all(response.as_bytes()).await;
        }
    }));
    port
}

fn trusting_localhost(port: u16) -> UrlPolicy {
    let mut policy = UrlPolicy::default();
    policy.trusted_hosts = vec!["localhost".to_owned()];
    policy.allowed_ports.push(port);
    policy.max_body_bytes = 16;
    policy
}

#[tokio::test]
async fn loopback_is_refused_unless_trusted() {
    let port = serve("HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok".to_owned()).await;
    let strict = SafeClient::new(UrlPolicy::default()).unwrap();
    let url = format!("http://127.0.0.1:{port}/");
    assert!(matches!(strict.get(&url).await, Err(SsrfError::Address(_))));

    let trusted = SafeClient::new(trusting_localhost(port)).unwrap();
    let body = trusted
        .get_trusted_bytes(&format!("http://localhost:{port}/"))
        .await
        .unwrap();
    assert_eq!(body, b"ok");
}

#[tokio::test]
async fn user_urls_cannot_reach_trusted_hosts() {
    // #270: a trusted host used to be exempt from every rule, port included.
    let port = serve("HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok".to_owned()).await;
    let client = SafeClient::new(trusting_localhost(port)).unwrap();
    for url in [
        format!("http://localhost:{port}/"),
        format!("http://LOCALHOST:{port}/"),
        "http://localhost:9999/admin".to_owned(),
    ] {
        assert!(
            client.get(&url).await.is_err(),
            "{url} reached a trusted host"
        );
    }
    let mut policy = UrlPolicy::default();
    policy.trusted_hosts = vec!["kroki".to_owned()];
    for url in ["http://kroki:8000/", "http://kroki:9999/", "http://kroki./"] {
        assert!(
            policy.check_url(&Url::parse(url).unwrap()).is_err(),
            "{url}"
        );
    }
    assert!(
        policy
            .check_trusted_url(&Url::parse("http://kroki:8000/").unwrap())
            .is_ok()
    );
    assert!(
        policy
            .check_trusted_url(&Url::parse("http://127.0.0.1/").unwrap())
            .is_err()
    );
}

#[tokio::test]
async fn redirects_never_lead_to_a_trusted_host() {
    // An external page answering `302 Location: http://kroki/...` must not
    // reach the sidecar. Simulated with a trusted service that redirects to
    // another trusted URL: the redirect hop is checked as user input.
    let target = serve("HTTP/1.1 200 OK\r\ncontent-length: 6\r\n\r\nsecret".to_owned()).await;
    let port = serve(format!(
        "HTTP/1.1 302 Found\r\nlocation: http://localhost:{target}/\r\ncontent-length: 0\r\n\r\n"
    ))
    .await;
    let mut policy = trusting_localhost(port);
    policy.allowed_ports.push(target);
    let client = SafeClient::new(policy).unwrap();
    let result = client
        .get_trusted_bytes(&format!("http://localhost:{port}/"))
        .await;
    assert!(
        result.is_err(),
        "followed a redirect to a trusted host: {result:?}"
    );
}

#[tokio::test]
async fn redirects_to_internal_addresses_are_refused() {
    let target = "http://127.0.0.1:9/";
    let port = serve(format!(
        "HTTP/1.1 302 Found\r\nlocation: {target}\r\ncontent-length: 0\r\n\r\n"
    ))
    .await;
    let client = SafeClient::new(trusting_localhost(port)).unwrap();
    let err = client
        .get_trusted(&format!("http://localhost:{port}/"))
        .await
        .unwrap_err();
    assert!(matches!(err, SsrfError::Http(_)), "{err:?}");
}

#[tokio::test]
async fn oversized_bodies_are_cut_off() {
    let port =
        serve("HTTP/1.1 200 OK\r\ncontent-length: 64\r\n\r\n".to_owned() + &"x".repeat(64)).await;
    let client = SafeClient::new(trusting_localhost(port)).unwrap();
    let err = client
        .get_trusted_bytes(&format!("http://localhost:{port}/"))
        .await
        .unwrap_err();
    assert!(matches!(err, SsrfError::TooLarge(16)));
}
