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
use vellum_api::server::{ServeConfig, Tls, serve_listener};

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn exchange<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    mut stream: S,
    path: &str,
) -> std::io::Result<String> {
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
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
    let gateway =
        GatewayBuilder::new(GatewayConfig::new("http://localhost", vec![1; 32]), "test").build()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let config = ServeConfig {
        bind: addr,
        tls,
        redirect_http: None,
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
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])?;
    let dir = std::env::temp_dir().join(format!("vellum-tls-{}", std::process::id()));
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
    let client = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
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
