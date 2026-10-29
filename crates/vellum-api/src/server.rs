// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Serving the gateway: TLS 1.3 with ACME certificates, HTTP/1.1 and
//! HTTP/2, and graceful drain.
//!
//! Three TLS modes cover the deployment ladder: certificates obtained and
//! renewed automatically over ACME (TLS-ALPN-01 on the HTTPS port, so no
//! port 80 is needed), certificate files from the operator, or plain HTTP
//! behind a TLS-terminating proxy or on localhost. Only TLS 1.3 is offered.

use std::future::Future;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::{ConnectInfo, Request};
use axum::response::{IntoResponse, Redirect, Response};
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig;
use rustls::crypto::CryptoProvider;
use rustls_pki_types::pem::PemObject as _;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{LazyConfigAcceptor, TlsAcceptor};
use tokio_stream::StreamExt as _;
use vellum_kernel::{Error, ErrorKind};

use crate::gateway::Gateway;

/// How the listener secures connections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tls {
    /// Plain HTTP: behind a proxy that terminates TLS, or on localhost.
    Off,
    /// PEM certificate chain and private key files.
    Files {
        /// The certificate chain.
        cert: PathBuf,
        /// The private key.
        key: PathBuf,
    },
    /// Certificates from an ACME directory (Let's Encrypt by default).
    Acme {
        /// The domains to certify (the app and public-pages hosts).
        domains: Vec<String>,
        /// Contact addresses, `mailto:` URLs.
        contact: Vec<String>,
        /// Where certificates and the account key are cached.
        cache_dir: PathBuf,
        /// The ACME directory URL; `None` uses Let's Encrypt production.
        directory: Option<String>,
    },
}

/// Where and how to listen.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    /// The address to listen on.
    pub bind: SocketAddr,
    /// The TLS mode.
    pub tls: Tls,
    /// An optional plain-HTTP address that redirects to HTTPS.
    pub redirect_http: Option<SocketAddr>,
}

fn io_error(what: &str, err: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::new(ErrorKind::Unavailable, format!("{what} failed")).with_source(err)
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

/// A TLS 1.3-only server configuration with HTTP/2 and HTTP/1.1 ALPN.
fn tls13(
    builder: impl FnOnce(
        rustls::ConfigBuilder<ServerConfig, rustls::WantsVerifier>,
    ) -> Result<ServerConfig, rustls::Error>,
) -> Result<Arc<ServerConfig>, Error> {
    let base = ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| io_error("TLS setup", e))?;
    let mut config = builder(base).map_err(|e| io_error("TLS setup", e))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Loads a certificate chain and key from PEM files.
///
/// # Errors
///
/// `unavailable` when a file cannot be read or holds no certificate or key.
pub fn load_pem(cert: &std::path::Path, key: &std::path::Path) -> Result<Arc<ServerConfig>, Error> {
    let chain = CertificateDer::pem_file_iter(cert)
        .map_err(|e| io_error("reading the TLS certificate", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| io_error("parsing the TLS certificate", e))?;
    if chain.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "the TLS certificate file holds no certificate",
        ));
    }
    let key = PrivateKeyDer::from_pem_file(key)
        .map_err(|e| io_error("reading the TLS private key", e))?;
    tls13(|b| b.with_no_client_auth().with_single_cert(chain, key))
}

enum Acceptor {
    Plain,
    Tls(TlsAcceptor),
    Acme {
        default: Arc<ServerConfig>,
        challenge: Arc<ServerConfig>,
    },
}

fn acme(
    domains: &[String],
    contact: &[String],
    cache_dir: &std::path::Path,
    directory: Option<&str>,
) -> Result<Acceptor, Error> {
    if domains.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "ACME needs at least one domain",
        ));
    }
    let mut config = rustls_acme::AcmeConfig::new_with_provider(domains, provider())
        .contact(contact)
        .cache(rustls_acme::caches::DirCache::new(cache_dir.to_path_buf()));
    config = match directory {
        Some(url) => config.directory(url),
        None => config.directory_lets_encrypt(true),
    };
    let mut state = config.state();
    let resolver = state.resolver();
    let default = tls13(|b| Ok(b.with_no_client_auth().with_cert_resolver(resolver)))?;
    let challenge = state.challenge_rustls_config_with_provider(provider());
    drop(tokio::spawn(async move {
        while let Some(event) = state.next().await {
            match event {
                Ok(ok) => tracing::info!(event = ?ok, "acme"),
                Err(err) => tracing::error!(error = ?err, "acme certificate renewal failed"),
            }
        }
    }));
    Ok(Acceptor::Acme { default, challenge })
}

/// Serves the gateway until `shutdown` resolves, then drains.
///
/// On shutdown, readiness fails at once, the listener stops accepting, and
/// in-flight requests get the gateway's drain timeout to finish.
///
/// # Errors
///
/// `unavailable` when the address cannot be bound or TLS cannot be set up.
pub async fn serve(
    gateway: Gateway,
    config: ServeConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Error> {
    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(|e| io_error("binding the listener", e))?;
    serve_listener(gateway, listener, config, shutdown).await
}

/// Like [`serve`], on a listener the caller bound (tests, socket activation).
///
/// # Errors
///
/// `unavailable` when TLS cannot be set up.
pub async fn serve_listener(
    gateway: Gateway,
    listener: TcpListener,
    config: ServeConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Error> {
    let acceptor = match &config.tls {
        Tls::Off => Acceptor::Plain,
        Tls::Files { cert, key } => Acceptor::Tls(TlsAcceptor::from(load_pem(cert, key)?)),
        Tls::Acme {
            domains,
            contact,
            cache_dir,
            directory,
        } => acme(domains, contact, cache_dir, directory.as_deref())?,
    };
    let local = listener
        .local_addr()
        .map_err(|e| io_error("reading the listener address", e))?;
    tracing::info!(address = %local, tls = !matches!(acceptor, Acceptor::Plain), "listening");
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    if let Some(redirect) = config.redirect_http {
        drop(tokio::spawn(redirect_to_https(redirect, stop_rx.clone())));
    }
    let router = gateway.router();
    let graceful = GracefulShutdown::new();
    let acceptor = Arc::new(acceptor);
    tokio::pin!(shutdown);
    loop {
        let (tcp, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(pair) => pair,
                Err(err) => {
                    tracing::warn!(error = %err, "accept failed");
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let service = tower::ServiceExt::map_request(
            router.clone(),
            move |mut request: http::Request<Incoming>| {
                let _ = request.extensions_mut().insert(ConnectInfo(peer));
                request
            },
        );
        let acceptor = Arc::clone(&acceptor);
        let watcher = graceful.watcher();
        drop(tokio::spawn(async move {
            if let Err(err) = connection(&acceptor, tcp, service, watcher).await {
                tracing::debug!(error = %err, peer = %peer, "connection ended with an error");
            }
        }));
    }
    let _ = stop_tx.send(true);
    gateway.start_draining();
    tracing::info!("shutting down: draining in-flight requests");
    tokio::select! {
        () = graceful.shutdown() => tracing::info!("drained"),
        () = tokio::time::sleep(gateway.drain_timeout()) => tracing::warn!("drain timed out; closing remaining connections"),
    }
    Ok(())
}

async fn connection<S>(
    acceptor: &Acceptor,
    tcp: TcpStream,
    service: S,
    watcher: hyper_util::server::graceful::Watcher,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    S: tower::Service<
            http::Request<Incoming>,
            Response = Response,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    let _ = tcp.set_nodelay(true);
    let service = TowerToHyperService::new(service);
    let builder = Builder::new(TokioExecutor::new());
    match acceptor {
        Acceptor::Plain => {
            watcher
                .watch(builder.serve_connection_with_upgrades(TokioIo::new(tcp), service))
                .await
        }
        Acceptor::Tls(tls) => {
            let stream = tls.accept(tcp).await?;
            watcher
                .watch(builder.serve_connection_with_upgrades(TokioIo::new(stream), service))
                .await
        }
        Acceptor::Acme { default, challenge } => {
            let start = LazyConfigAcceptor::new(rustls::server::Acceptor::default(), tcp).await?;
            if rustls_acme::is_tls_alpn_challenge(&start.client_hello()) {
                tracing::info!("answering an ACME TLS-ALPN-01 challenge");
                let mut tls = start.into_stream(Arc::clone(challenge)).await?;
                tokio::io::AsyncWriteExt::shutdown(&mut tls).await?;
                return Ok(());
            }
            let stream = start.into_stream(Arc::clone(default)).await?;
            watcher
                .watch(builder.serve_connection_with_upgrades(TokioIo::new(stream), service))
                .await
        }
    }
}

async fn redirect_to_https(bind: SocketAddr, mut stop: tokio::sync::watch::Receiver<bool>) {
    let listener = match TcpListener::bind(bind).await {
        Ok(l) => l,
        Err(err) => {
            tracing::error!(error = %err, address = %bind, "could not bind the HTTP redirect listener");
            return;
        }
    };
    let router = Router::new().fallback(redirect);
    loop {
        let tcp = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((tcp, _)) => tcp,
                Err(_) => continue,
            },
            _ = stop.changed() => return,
        };
        let service = TowerToHyperService::new(router.clone());
        drop(tokio::spawn(async move {
            let _ = Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(tcp), service)
                .await;
        }));
    }
}

async fn redirect(request: Request) -> Response {
    let host = request
        .headers()
        .get(http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return http::StatusCode::BAD_REQUEST.into_response();
    }
    let path = request
        .uri()
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    Redirect::permanent(&format!("https://{host}{path}")).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn redirects_keep_host_and_path() {
        let req = Request::get("/a/b?c=1")
            .header("host", "vellum.northstar.dev:80")
            .body(axum::body::Body::empty());
        let Ok(req) = req else { return };
        let resp = redirect(req).await;
        assert_eq!(resp.status(), http::StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            resp.headers().get("location").and_then(|v| v.to_str().ok()),
            Some("https://vellum.northstar.dev/a/b?c=1")
        );
        let Ok(bad) = Request::get("/")
            .header("host", "evil\"x")
            .body(axum::body::Body::empty())
        else {
            return;
        };
        assert_eq!(redirect(bad).await.status(), http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn missing_files_are_reported() {
        let err = load_pem(
            std::path::Path::new("/nonexistent/cert.pem"),
            std::path::Path::new("/nonexistent/key.pem"),
        );
        assert!(err.is_err());
    }
}
