// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The real listener: plain HTTP, TLS 1.3 from files, and graceful drain.

use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::GatewayBuilder;
use vellum_api::server::{ConnectionLimits, ServeConfig, Tls, serve_listener};

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn exchange<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    mut stream: S,
    path: &str,
) -> std::io::Result<String> {
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    let mut out = Vec::new();
    let _ = stream.read_to_end(&mut out).await?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

async fn start(
    tls: Tls,
) -> Result<
    (
        std::net::SocketAddr,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error>,
> {
    start_with(tls, ConnectionLimits::default()).await
}

async fn start_with(
    tls: Tls,
    limits: ConnectionLimits,
) -> Result<
    (
        std::net::SocketAddr,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error>,
> {
    start_gateway(
        GatewayBuilder::new(
            GatewayConfig::new("http://localhost", vec![1; 32]),
            "test",
        ),
        tls,
        limits,
    )
    .await
}

async fn start_gateway(
    builder: GatewayBuilder,
    tls: Tls,
    limits: ConnectionLimits,
) -> Result<
    (
        std::net::SocketAddr,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error>,
> {
    let gateway = builder.build()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let config = ServeConfig {
        bind: addr,
        tls,
        redirect_http: None,
        limits,
    };
    let handle = tokio::spawn(async move {
        let _ = serve_listener(gateway, listener, config, async move {
            let _ = rx.await;
        })
        .await;
    });
    Ok((addr, tx, handle))
}

#[tokio::test]
async fn plain_http_and_graceful_shutdown() -> TestResult {
    let (addr, stop, handle) = start(Tls::Off).await?;
    let reply = exchange(TcpStream::connect(addr).await?, "/healthz").await?;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.to_ascii_lowercase().contains("x-request-id:"));
    let _ = stop.send(());
    tokio::time::timeout(Duration::from_secs(5), handle).await??;
    assert!(
        TcpStream::connect(addr).await.is_err(),
        "listener closed after drain"
    );
    Ok(())
}

#[tokio::test]
async fn tls_files_negotiate_tls_1_3_only() -> TestResult {
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])?;
    let dir =
        std::env::temp_dir().join(format!("vellum-tls-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let (cert_path, key_path) = (dir.join("cert.pem"), dir.join("key.pem"));
    std::fs::write(&cert_path, cert.cert.pem())?;
    std::fs::write(&key_path, cert.key_pair.serialize_pem())?;
    let (addr, stop, handle) = start(Tls::Files {
        cert: cert_path,
        key: key_path,
    })
    .await?;

    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(cert.cert.der().to_vec()))?;
    // A client that also offers TLS 1.2 still gets TLS 1.3.
    let client =
        rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots.clone())
            .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
    let stream = connector
        .connect(
            ServerName::try_from("localhost")?,
            TcpStream::connect(addr).await?,
        )
        .await?;
    assert_eq!(
        stream.get_ref().1.protocol_version(),
        Some(rustls::ProtocolVersion::TLSv1_3)
    );
    let reply = exchange(stream, "/healthz").await?;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");

    // A TLS 1.2-only client is refused.
    let old = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS12])?
        .with_root_certificates(roots)
        .with_no_client_auth();
    let refused = tokio_rustls::TlsConnector::from(Arc::new(old))
        .connect(
            ServerName::try_from("localhost")?,
            TcpStream::connect(addr).await?,
        )
        .await;
    assert!(refused.is_err());

    let _ = stop.send(());
    tokio::time::timeout(Duration::from_secs(5), handle).await??;
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

/// Short limits so the hostile-client tests finish quickly.
const fn tight(max_connections: usize) -> ConnectionLimits {
    ConnectionLimits {
        handshake_timeout: Duration::from_millis(300),
        header_read_timeout: Duration::from_millis(300),
        max_connections,
        max_connections_per_ip: 0,
        body_timeout: Duration::from_secs(1),
        idle_timeout: Duration::from_millis(600),
        upload_prefixes: &[],
        upload_rate: 0,
    }
}

/// True when the server has closed the stream (or reset it).
async fn closed_within(stream: &mut TcpStream, wait: Duration) -> bool {
    let mut buf = [0_u8; 512];
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match tokio::time::timeout_at(deadline, stream.read(&mut buf)).await {
            Err(_) => return false,
            Ok(Ok(0) | Err(_)) => return true,
            // A 408 or similar before the close is fine; keep reading.
            Ok(Ok(_)) => {}
        }
    }
}

// Each of these clients used to hold its connection forever, so one
// attacker could use up every file descriptor (#354).

#[tokio::test]
async fn a_silent_connection_is_closed() -> TestResult {
    let (addr, _stop, _handle) = start_with(Tls::Off, tight(16)).await?;
    let mut silent = TcpStream::connect(addr).await?;
    assert!(closed_within(&mut silent, Duration::from_secs(3)).await);
    Ok(())
}

#[tokio::test]
async fn half_sent_headers_are_closed() -> TestResult {
    let (addr, _stop, _handle) = start_with(Tls::Off, tight(16)).await?;
    let mut slow = TcpStream::connect(addr).await?;
    slow.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\n")
        .await?;
    assert!(closed_within(&mut slow, Duration::from_secs(3)).await);
    Ok(())
}

#[tokio::test]
async fn an_idle_keep_alive_connection_is_closed() -> TestResult {
    let (addr, _stop, _handle) = start_with(Tls::Off, tight(16)).await?;
    let mut idle = TcpStream::connect(addr).await?;
    idle.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\n\r\n")
        .await?;
    let mut buf = [0_u8; 1024];
    let n = idle.read(&mut buf).await?;
    assert!(buf[..n].starts_with(b"HTTP/1.1 200"));
    assert!(closed_within(&mut idle, Duration::from_secs(3)).await);
    Ok(())
}

#[tokio::test]
async fn a_stalled_tls_handshake_is_closed() -> TestResult {
    let cert =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])?;
    let dir = std::env::temp_dir()
        .join(format!("vellum-tls-stall-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let (cert_path, key_path) = (dir.join("cert.pem"), dir.join("key.pem"));
    std::fs::write(&cert_path, cert.cert.pem())?;
    std::fs::write(&key_path, cert.key_pair.serialize_pem())?;
    let tls = Tls::Files {
        cert: cert_path,
        key: key_path,
    };
    let (addr, _stop, _handle) = start_with(tls, tight(16)).await?;
    let mut stalled = TcpStream::connect(addr).await?;
    // The first bytes of a ClientHello record, and nothing more.
    stalled.write_all(&[0x16, 0x03, 0x01, 0x02, 0x00]).await?;
    assert!(closed_within(&mut stalled, Duration::from_secs(3)).await);
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

#[tokio::test]
async fn hostile_clients_cannot_lock_others_out() -> TestResult {
    let (addr, _stop, _handle) = start_with(Tls::Off, tight(4)).await?;
    // More half-open connections than the server will hold at once.
    let mut hostile = Vec::new();
    for _ in 0..12 {
        let mut s = TcpStream::connect(addr).await?;
        let _ = s.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\n").await;
        hostile.push(s);
    }
    // A real client still gets an answer once the stalled ones time out.
    let reply = tokio::time::timeout(
        Duration::from_secs(5),
        exchange(TcpStream::connect(addr).await?, "/healthz"),
    )
    .await??;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    drop(hostile);
    Ok(())
}

// Red team round 4: slow bodies, idle HTTP/2 and one client hogging the
// connection slots.

/// A gateway with `/echo`, which reads the whole body and answers its length.
async fn start_echo(
    limits: ConnectionLimits,
) -> Result<
    (
        std::net::SocketAddr,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ),
    Box<dyn std::error::Error>,
> {
    let echo = axum::Router::new().route(
        "/",
        axum::routing::post(|body: axum::body::Bytes| async move {
            body.len().to_string()
        }),
    );
    let builder = GatewayBuilder::new(
        GatewayConfig::new("http://localhost", vec![1; 32]),
        "test",
    )
    .mount_cross_site("/echo", echo);
    start_gateway(builder, Tls::Off, limits).await
}

const SLOW_POST: &[u8] =
    b"POST /echo HTTP/1.1\r\nHost: x\r\nContent-Type: text/plain\r\n\
        Content-Length: 100000\r\n\r\n";

#[tokio::test]
async fn a_body_that_stops_arriving_is_cut_off() -> TestResult {
    let (addr, _stop, _handle) = start_echo(tight(16)).await?;
    let mut slow = TcpStream::connect(addr).await?;
    slow.write_all(SLOW_POST).await?;
    slow.write_all(b"only a little").await?;
    assert!(closed_within(&mut slow, Duration::from_secs(3)).await);
    Ok(())
}

#[tokio::test]
async fn a_trickled_body_is_cut_off_at_the_deadline() -> TestResult {
    let (addr, _stop, _handle) = start_echo(tight(16)).await?;
    let mut slow = TcpStream::connect(addr).await?;
    slow.write_all(SLOW_POST).await?;
    // One byte every 100 ms never trips the 300 ms gap, but the whole
    // body must arrive within 1 s.
    let started = tokio::time::Instant::now();
    let mut buf = [0_u8; 256];
    loop {
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "the trickle was never cut off"
        );
        if slow.write_all(b"x").await.is_err() {
            break;
        }
        match tokio::time::timeout(
            Duration::from_millis(100),
            slow.read(&mut buf),
        )
        .await
        {
            Ok(Ok(0) | Err(_)) => break,
            Ok(Ok(_)) => {
                assert!(
                    !buf.starts_with(b"HTTP/1.1 200"),
                    "a trickled body was accepted"
                );
                break;
            }
            Err(_) => {}
        }
    }
    assert!(started.elapsed() >= Duration::from_millis(900));
    Ok(())
}

#[tokio::test]
async fn a_prompt_body_is_read_in_full() -> TestResult {
    let (addr, _stop, _handle) = start_echo(tight(16)).await?;
    let mut fast = TcpStream::connect(addr).await?;
    let body = vec![b'n'; 100_000];
    fast.write_all(SLOW_POST).await?;
    fast.write_all(&body).await?;
    let mut buf = vec![0_u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(3), fast.read(&mut buf))
        .await??;
    let reply = String::from_utf8_lossy(&buf[..n]);
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.ends_with("100000"), "{reply}");
    Ok(())
}

/// Speaks just enough HTTP/2 (prior knowledge, no TLS) to hold a
/// connection open: the preface, empty settings, and an answer to every
/// ping. Returns true if the server closed the connection within `wait`.
async fn idle_h2_closed_within(
    addr: std::net::SocketAddr,
    wait: Duration,
) -> std::io::Result<bool> {
    let mut stream = TcpStream::connect(addr).await?;
    stream
        .write_all(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n\0\0\0\x04\0\0\0\0\0")
        .await?;
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let mut head = [0_u8; 9];
        match tokio::time::timeout_at(deadline, stream.read_exact(&mut head))
            .await
        {
            Err(_) => return Ok(false),
            Ok(Err(_)) => return Ok(true),
            Ok(Ok(_)) => {}
        }
        let len = usize::from(head[0]) << 16
            | usize::from(head[1]) << 8
            | usize::from(head[2]);
        let mut payload = vec![0_u8; len];
        if stream.read_exact(&mut payload).await.is_err() {
            return Ok(true);
        }
        match (head[3], head[4] & 1) {
            // A GOAWAY means the server is closing.
            (7, _) => return Ok(true),
            // SETTINGS: acknowledge.
            (4, 0) => stream.write_all(&[0, 0, 0, 4, 1, 0, 0, 0, 0]).await?,
            // PING: answer it, as a live client would.
            (6, 0) => {
                let mut ack = vec![0, 0, 8, 6, 1, 0, 0, 0, 0];
                ack.extend_from_slice(&payload);
                stream.write_all(&ack).await?;
            }
            _ => {}
        }
    }
}

#[tokio::test]
async fn an_idle_http2_connection_is_closed() -> TestResult {
    let (addr, _stop, _handle) = start_with(Tls::Off, tight(16)).await?;
    // hyper pings every 300 ms here and the client answers each ping, so
    // only the idle limit (600 ms) can close it.
    assert!(idle_h2_closed_within(addr, Duration::from_secs(4)).await?);
    Ok(())
}

#[tokio::test]
async fn one_client_cannot_take_every_slot() -> TestResult {
    let mut limits = tight(64);
    limits.max_connections_per_ip = 2;
    limits.header_read_timeout = Duration::from_secs(5);
    limits.handshake_timeout = Duration::from_secs(5);
    let (addr, _stop, _handle) = start_with(Tls::Off, limits).await?;
    let mut held = Vec::new();
    for _ in 0..2 {
        let mut s = TcpStream::connect(addr).await?;
        s.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\n").await?;
        held.push(s);
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    // A third connection from the same address is closed at once.
    let mut third = TcpStream::connect(addr).await?;
    let _ = third
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\n\r\n")
        .await;
    assert!(closed_within(&mut third, Duration::from_secs(1)).await);
    // Once one is let go, the client may connect again.
    drop(held.pop());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let reply = tokio::time::timeout(
        Duration::from_secs(3),
        exchange(TcpStream::connect(addr).await?, "/healthz"),
    )
    .await??;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    Ok(())
}
