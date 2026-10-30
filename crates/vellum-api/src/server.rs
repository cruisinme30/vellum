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
use std::time::Duration;

use axum::Router;
use axum::extract::{ConnectInfo, Request};
use axum::response::{IntoResponse, Redirect, Response};
use hyper::body::Incoming;
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig;
use rustls::crypto::CryptoProvider;
use rustls_pki_types::pem::PemObject as _;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio_rustls::{LazyConfigAcceptor, TlsAcceptor};
use tokio_stream::StreamExt as _;
use vellum_kernel::{Error, ErrorKind};

use crate::deadline::{Activity, Deadline, Tracked};
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
    /// How long and how many connections a client may hold.
    pub limits: ConnectionLimits,
}

/// Bounds on what one connection may hold, so a client that never
/// finishes a request cannot use up the server's file descriptors (#354).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectionLimits {
    /// Time from accepting a connection to its first byte (plain HTTP) or
    /// to a finished TLS handshake.
    pub handshake_timeout: Duration,
    /// Time to receive a request's headers, and the longest an HTTP/1.1
    /// connection may sit idle between requests.
    pub header_read_timeout: Duration,
    /// Connections served at once. Further connections wait in the
    /// kernel's accept queue until one closes.
    pub max_connections: usize,
    /// Connections one client address (one `IPv6` `/64`) may hold at once;
    /// 0 for no cap. Leave it off behind a proxy, where every connection
    /// comes from the proxy's address.
    pub max_connections_per_ip: usize,
    /// Time for a whole request body to arrive. Gaps between its chunks
    /// may be at most `header_read_timeout`.
    pub body_timeout: Duration,
    /// How long a connection may hold no request before it is closed.
    /// For HTTP/2, where the header timer does not apply, this is what
    /// frees a connection whose client only answers pings.
    pub idle_timeout: Duration,
    /// The upload routes' path prefixes. A body sent there with a
    /// `Content-Length` gets `body_timeout` plus the time to send that
    /// many bytes at `upload_rate`, so a large file on a slow link
    /// finishes; gaps are still bounded by `header_read_timeout` (D213).
    pub upload_prefixes: &'static [&'static str],
    /// The slowest average rate, in bytes a second, an upload is given
    /// time for.
    pub upload_rate: u64,
}

/// The most extra time an upload body is given.
const MAX_UPLOAD_EXTRA: Duration = Duration::from_mins(30);

impl ConnectionLimits {
    /// The time a request body to `path` declaring `length` bytes may take.
    #[must_use]
    pub fn body_time(&self, path: &str, length: Option<u64>) -> Duration {
        let upload = self.upload_prefixes.iter().any(|prefix| {
            path.strip_prefix(prefix)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        });
        match length {
            // The route's own body limit refuses a larger declared length
            // before the body is read; the cap only bounds a lie.
            Some(length) if upload && self.upload_rate > 0 => self.body_timeout.saturating_add(
                Duration::from_secs(length / self.upload_rate).min(MAX_UPLOAD_EXTRA),
            ),
            _ => self.body_timeout,
        }
    }
}

impl Default for ConnectionLimits {
    fn default() -> Self {
        Self {
            handshake_timeout: Duration::from_secs(10),
            header_read_timeout: Duration::from_secs(15),
            max_connections: 4096,
            max_connections_per_ip: 0,
            body_timeout: Duration::from_mins(2),
            idle_timeout: Duration::from_mins(1),
            upload_prefixes: &[],
            upload_rate: 512 * 1024,
        }
    }
}

/// Connections held per client address, for
/// [`ConnectionLimits::max_connections_per_ip`].
#[derive(Debug, Default)]
struct PerClient {
    held: std::sync::Mutex<std::collections::HashMap<std::net::IpAddr, usize>>,
}

impl PerClient {
    /// Counts a connection from `ip`, or `None` when it already holds `cap`.
    fn enter(self: &Arc<Self>, ip: std::net::IpAddr, cap: usize) -> Option<Held> {
        let ip = vellum_security::rate::client_bucket(ip);
        let mut held = self.held.lock().ok()?;
        let count = held.entry(ip).or_insert(0);
        let full = cap > 0 && *count >= cap;
        if !full {
            *count += 1;
        }
        drop(held);
        (!full).then(|| Held(Arc::clone(self), ip))
    }
}

/// One counted connection; see [`PerClient::enter`].
struct Held(Arc<PerClient>, std::net::IpAddr);

impl Drop for Held {
    fn drop(&mut self) {
        if let Ok(mut held) = self.0.held.lock()
            && let Some(count) = held.get_mut(&self.1)
        {
            *count = count.saturating_sub(1);
            if *count == 0 {
                let _ = held.remove(&self.1);
            }
        }
    }
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
        drop(tokio::spawn(redirect_to_https(
            redirect,
            config.limits,
            stop_rx.clone(),
        )));
    }
    let router = gateway.router();
    let graceful = GracefulShutdown::new();
    let acceptor = Arc::new(acceptor);
    let limits = config.limits;
    let cap = fit_to_open_file_limit(limits.max_connections);
    let slots = Arc::new(Semaphore::new(cap));
    let clients = Arc::new(PerClient::default());
    let mut accept_errors = AcceptErrors::default();
    tokio::pin!(shutdown);
    loop {
        // Wait for a free slot before accepting, so extra connections queue
        // in the kernel instead of using up file descriptors.
        let slot = tokio::select! {
            slot = Arc::clone(&slots).acquire_owned() => match slot {
                Ok(slot) => slot,
                Err(_) => break,
            },
            () = &mut shutdown => break,
        };
        let (tcp, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(pair) => {
                    accept_errors.recovered();
                    pair
                }
                Err(err) => {
                    // Out of file descriptors, or a similar resource error:
                    // retrying at once spins a core and floods the log.
                    accept_errors.failed(&err);
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                    continue;
                }
            },
            () = &mut shutdown => break,
        };
        let Some(held) = clients.enter(peer.ip(), limits.max_connections_per_ip) else {
            tracing::info!(event = "gateway.client_connection_cap", peer = %peer.ip(), "refused a connection: this client holds too many");
            drop(tcp);
            continue;
        };
        let activity = Activity::new();
        let service = per_connection(router.clone(), peer, limits, Arc::clone(&activity));
        let acceptor = Arc::clone(&acceptor);
        let watcher = graceful.watcher();
        drop(tokio::spawn(async move {
            if let Err(err) = connection(&acceptor, tcp, service, watcher, limits, &activity).await
            {
                tracing::debug!(error = %err, peer = %peer, "connection ended with an error");
            }
            drop(held);
            drop(slot);
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

/// The router as one connection sees it: the peer address in each
/// request, bodies under [`Deadline`], and every request counted in
/// `activity` until its response is sent.
fn per_connection(
    router: Router,
    peer: SocketAddr,
    limits: ConnectionLimits,
    activity: Arc<Activity>,
) -> impl tower::Service<
    http::Request<Incoming>,
    Response = Response,
    Error = std::convert::Infallible,
    Future = impl Future<Output = Result<Response, std::convert::Infallible>> + Send,
> + Clone
+ Send
+ 'static {
    tower::service_fn(move |request: http::Request<Incoming>| {
        let busy = activity.begin();
        let (mut parts, body) = request.into_parts();
        let _ = parts.extensions.insert(ConnectInfo(peer));
        let length = parts
            .headers
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let body = axum::body::Body::new(Deadline::new(
            body,
            limits.header_read_timeout,
            limits.body_time(parts.uri.path(), length),
        ));
        let router = router.clone();
        async move {
            let response =
                tower::ServiceExt::oneshot(router, http::Request::from_parts(parts, body)).await?;
            Ok(response.map(|body| axum::body::Body::new(Tracked::new(body, busy))))
        }
    })
}

/// Runs `conn` until it ends, or until it has held no request for
/// `idle`; then it is dropped, which closes the socket.
async fn until_idle<E>(
    conn: impl Future<Output = Result<(), E>>,
    activity: &Activity,
    idle: Duration,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
where
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    tokio::pin!(conn);
    let mut tick = tokio::time::interval((idle / 4).max(Duration::from_millis(10)));
    loop {
        tokio::select! {
            done = &mut conn => return done.map_err(Into::into),
            _ = tick.tick() => {
                if activity.idle_for() >= idle {
                    tracing::debug!("closing a connection that held no request");
                    return Ok(());
                }
            }
        }
    }
}

async fn connection<S>(
    acceptor: &Acceptor,
    tcp: TcpStream,
    service: S,
    watcher: hyper_util::server::graceful::Watcher,
    limits: ConnectionLimits,
    activity: &Activity,
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
    let builder = builder(limits);
    let handshake = limits.handshake_timeout;
    let idle = limits.idle_timeout;
    match acceptor {
        Acceptor::Plain => {
            // A client that connects and sends nothing never reaches hyper's
            // header timer, so wait for its first byte here.
            let mut first = [0_u8; 1];
            match tokio::time::timeout(handshake, tcp.peek(&mut first)).await {
                Ok(Ok(n)) if n > 0 => {}
                Ok(Ok(_)) => return Ok(()),
                Ok(Err(err)) => return Err(err.into()),
                Err(_) => return Err("no request before the handshake timeout".into()),
            }
            let conn = builder.serve_connection_with_upgrades(TokioIo::new(tcp), service);
            until_idle(watcher.watch(conn), activity, idle).await
        }
        Acceptor::Tls(tls) => {
            let stream = tokio::time::timeout(handshake, tls.accept(tcp))
                .await
                .map_err(|_| "TLS handshake timed out")??;
            let conn = builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
            until_idle(watcher.watch(conn), activity, idle).await
        }
        Acceptor::Acme { default, challenge } => {
            let start = tokio::time::timeout(
                handshake,
                LazyConfigAcceptor::new(rustls::server::Acceptor::default(), tcp),
            )
            .await
            .map_err(|_| "TLS handshake timed out")??;
            if rustls_acme::is_tls_alpn_challenge(&start.client_hello()) {
                tracing::info!("answering an ACME TLS-ALPN-01 challenge");
                let mut tls =
                    tokio::time::timeout(handshake, start.into_stream(Arc::clone(challenge)))
                        .await
                        .map_err(|_| "TLS handshake timed out")??;
                tokio::io::AsyncWriteExt::shutdown(&mut tls).await?;
                return Ok(());
            }
            let stream = tokio::time::timeout(handshake, start.into_stream(Arc::clone(default)))
                .await
                .map_err(|_| "TLS handshake timed out")??;
            let conn = builder.serve_connection_with_upgrades(TokioIo::new(stream), service);
            until_idle(watcher.watch(conn), activity, idle).await
        }
    }
}

/// File descriptors kept free for the database, the store and logs.
const RESERVED_FILES: u64 = 256;

/// Raises the soft open-file limit to the hard one, then returns the
/// connection cap that fits under it, so the cap is reached before the
/// process runs out of descriptors (#354).
#[cfg(unix)]
fn fit_to_open_file_limit(wanted: usize) -> usize {
    use rustix::process::{Resource, getrlimit, setrlimit};
    let mut limit = getrlimit(Resource::Nofile);
    if limit.current.is_some() && limit.current != limit.maximum {
        let raised = rustix::process::Rlimit {
            current: limit.maximum,
            maximum: limit.maximum,
        };
        // Refused on some systems (macOS will not take "unlimited"); the
        // cap then fits the current limit.
        if setrlimit(Resource::Nofile, raised).is_ok() {
            limit = getrlimit(Resource::Nofile);
        }
    }
    let cap = limit.current.map_or(wanted, |soft| {
        let room = soft.saturating_sub(RESERVED_FILES).max(1);
        wanted.min(usize::try_from(room).unwrap_or(usize::MAX))
    });
    if cap < wanted {
        tracing::warn!(cap, wanted, open_files = ?limit.current, "the open-file limit lowers the connection cap");
    }
    cap.max(1)
}

#[cfg(not(unix))]
const fn fit_to_open_file_limit(wanted: usize) -> usize {
    if wanted == 0 { 1 } else { wanted }
}

/// How long the accept loop pauses after a failed accept.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// Logs accept failures once per burst instead of once per retry.
#[derive(Default)]
struct AcceptErrors {
    failing: u64,
}

impl AcceptErrors {
    fn failed(&mut self, err: &std::io::Error) {
        if self.failing == 0 {
            tracing::warn!(error = %err, "accept failed; pausing new connections");
        }
        self.failing = self.failing.saturating_add(1);
    }

    fn recovered(&mut self) {
        if self.failing > 0 {
            tracing::warn!(failed = self.failing, "accepting connections again");
            self.failing = 0;
        }
    }
}

/// An HTTP/1.1 and HTTP/2 connection builder with the timeouts in `limits`.
fn builder(limits: ConnectionLimits) -> Builder<TokioExecutor> {
    let mut builder = Builder::new(TokioExecutor::new());
    let _ = builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(limits.header_read_timeout);
    let _ = builder
        .http2()
        .timer(TokioTimer::new())
        .keep_alive_interval(limits.header_read_timeout)
        .keep_alive_timeout(limits.handshake_timeout);
    builder
}

async fn redirect_to_https(
    bind: SocketAddr,
    limits: ConnectionLimits,
    mut stop: tokio::sync::watch::Receiver<bool>,
) {
    let listener = match TcpListener::bind(bind).await {
        Ok(l) => l,
        Err(err) => {
            tracing::error!(error = %err, address = %bind, "could not bind the HTTP redirect listener");
            return;
        }
    };
    let router = Router::new().fallback(redirect);
    let slots = Arc::new(Semaphore::new(limits.max_connections.max(1)));
    let mut accept_errors = AcceptErrors::default();
    loop {
        let slot = tokio::select! {
            slot = Arc::clone(&slots).acquire_owned() => match slot {
                Ok(slot) => slot,
                Err(_) => return,
            },
            _ = stop.changed() => return,
        };
        let tcp = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((tcp, _)) => {
                    accept_errors.recovered();
                    tcp
                }
                Err(err) => {
                    accept_errors.failed(&err);
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                    continue;
                }
            },
            _ = stop.changed() => return,
        };
        let service = TowerToHyperService::new(router.clone());
        drop(tokio::spawn(async move {
            // The redirect answers one short request; the header timer
            // also closes connections that send nothing.
            let _ = tokio::time::timeout(
                limits.handshake_timeout + limits.header_read_timeout,
                builder(limits).serve_connection(TokioIo::new(tcp), service),
            )
            .await;
            drop(slot);
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

    #[test]
    fn uploads_get_time_for_their_size() {
        let limits = ConnectionLimits {
            upload_prefixes: &["/api/v1/attachments"],
            ..ConnectionLimits::default()
        };
        let base = limits.body_timeout;
        let mib100 = 100 * 1024 * 1024;
        assert_eq!(
            limits.body_time("/api/v1/attachments/01J", Some(mib100)),
            base + Duration::from_secs(200)
        );
        // Other routes, unknown lengths and lies keep their bounds.
        assert_eq!(limits.body_time("/api/v1/books", Some(mib100)), base);
        assert_eq!(limits.body_time("/api/v1/attachmentsX", Some(mib100)), base);
        assert_eq!(limits.body_time("/api/v1/attachments/01J", None), base);
        assert_eq!(
            limits.body_time("/api/v1/attachments/01J", Some(u64::MAX)),
            base + MAX_UPLOAD_EXTRA
        );
    }

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
