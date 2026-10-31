// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum serve`: configuration, the gateway and the listener.
//!
//! Server settings come from the kernel configuration (data folder, bind
//! address, public URL). Edge settings are `VELLUM_*` variables:
//!
//! | Variable | Meaning |
//! | --- | --- |
//! | `VELLUM_PUBLIC_PAGES_URL` | Origin for public pages and share links |
//! | `VELLUM_TLS` | `off` (default), `files` or `acme` |
//! | `VELLUM_TLS_CERT`, `VELLUM_TLS_KEY` | PEM files for `files` |
//! | `VELLUM_ACME_DOMAINS` | Comma-separated; default: the app and pages hosts |
//! | `VELLUM_ACME_CONTACT` | Comma-separated e-mail addresses |
//! | `VELLUM_ACME_DIRECTORY` | ACME directory URL; default Let's Encrypt |
//! | `VELLUM_HTTP_REDIRECT` | Address of a plain-HTTP listener redirecting to HTTPS |
//! | `VELLUM_CSRF_KEY` | Base64 key shared by replicas; default a per-install key file |
//! | `VELLUM_CSRF_KEY_FILE` | A file holding that base64 key (a Docker or Kubernetes secret) |
//! | `VELLUM_EMBED_ORIGINS` | Comma-separated origins allowed in sandboxed embeds |
//! | `VELLUM_CLAMD_ADDRESS` | `host:port` of `ClamAV`, the default upload virus scanner |
//! | `VELLUM_MAX_CONNECTIONS_PER_IP` | Connections one client address may hold; default 256 with TLS on, no cap with it off |
//! | `VELLUM_INDEX_KEY` | 64 hex characters: the key of the SQLite index |
//! | `VELLUM_INDEX_PLAINTEXT` | `1` opens the SQLite index without encryption |
//! | `VELLUM_KMS_KEY` | The files' KEK in a key service: `aws-kms:…`, `gcp-kms:…` or `vault:…` |
//! | `VELLUM_KEK_PASSPHRASE`, `VELLUM_KEK_PASSPHRASE_FILE` | Derive the files' KEK from a passphrase |
//! | `VELLUM_FILES_PLAINTEXT` | `1` stores the workspace files without encryption |
//! | `VELLUM_OIDC_ISSUER`, `VELLUM_OIDC_CLIENT_ID` | Turn on single sign-on with this `OpenID Connect` provider |
//! | `VELLUM_OIDC_CLIENT_SECRET` | The client secret, for a confidential client |
//! | `VELLUM_OIDC_NAME` | The provider's name on the sign-in button |
//! | `VELLUM_OIDC_ALLOWED_DOMAINS` | Comma-separated email domains that get an account on first sign-in |
//! | `VELLUM_OIDC_AUTO_CREATE` | `false` to sign in existing accounts only |
//!
//! With none of these, the SQLite index key and the files' key are made on
//! the first start and kept in `.vellum/keys`, so `vellum serve` needs no
//! settings (see [`crate::at_rest`]).

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use vellum_api::config::{GatewayConfig, host_of};
use vellum_api::gateway::{Gateway, GatewayBuilder};

use crate::observability::Observability;
use vellum_api::server::{ServeConfig, Tls, serve};
use vellum_db::IndexConfig;
use vellum_identity::Store;
use vellum_kernel::{Config, Error, ErrorKind};
use vellum_security::upload::Clamd;

/// Connections one client address may hold when Vellum terminates TLS.
const PER_IP_CONNECTIONS: usize = 256;

/// How long one virus scan may take.
const CLAMD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

use crate::{services, web};

fn invalid(key: &str, why: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::InvalidInput, format!("{key}: {why}"))
}

fn list(value: Option<String>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Edge settings derived from the kernel configuration and variables.
#[derive(Debug)]
pub struct Edge {
    /// The gateway configuration (without its CSRF key, filled in later).
    pub gateway: GatewayConfig,
    /// The listener configuration.
    pub serve: ServeConfig,
    /// A CSRF key given in the environment, if any.
    pub csrf_key: Option<Vec<u8>>,
    /// The default upload virus scanner (`VELLUM_CLAMD_ADDRESS`); an
    /// address set by an admin takes precedence.
    pub scanner: Option<Clamd>,
}

/// The app's origin when no public URL is set: the bind address, or, for
/// an all-interfaces bind such as Docker's `0.0.0.0:7700`, the loopback
/// address on the same port. A browser can never send `Origin:
/// http://0.0.0.0:7700`, so that origin rejected every sign-up as
/// cross-origin. Any other address needs `VELLUM_PUBLIC_URL`.
fn default_origin(bind: std::net::SocketAddr) -> String {
    if !bind.ip().is_unspecified() {
        return format!("http://{bind}");
    }
    let loopback = std::net::SocketAddr::new(
        match bind.ip() {
            std::net::IpAddr::V4(_) => std::net::Ipv4Addr::LOCALHOST.into(),
            std::net::IpAddr::V6(_) => std::net::Ipv6Addr::LOCALHOST.into(),
        },
        bind.port(),
    );
    tracing::warn!(
        %bind,
        origin = %format!("http://{loopback}"),
        "VELLUM_PUBLIC_URL is not set; the app accepts sign-ins only \
         from this address. Set VELLUM_PUBLIC_URL to the address \
         people open."
    );
    format!("http://{loopback}")
}

/// Builds the edge settings from `config` and a variable lookup.
///
/// # Errors
///
/// `invalid_input` naming the variable that is wrong.
pub fn edge(
    config: &Config,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Edge, Error> {
    let app_origin = config
        .public_url
        .clone()
        .unwrap_or_else(|| default_origin(config.bind));
    let mut gateway = GatewayConfig::new(app_origin, Vec::new());
    gateway.public_origin = var("VELLUM_PUBLIC_PAGES_URL")
        .map(|u| u.trim_end_matches('/').to_owned());
    gateway.embed_origins = list(var("VELLUM_EMBED_ORIGINS"));
    let tls = match var("VELLUM_TLS").as_deref().unwrap_or("off") {
        "off" => Tls::Off,
        "files" => Tls::Files {
            cert: var("VELLUM_TLS_CERT").map(PathBuf::from).ok_or_else(
                || invalid("VELLUM_TLS_CERT", "required when VELLUM_TLS=files"),
            )?,
            key: var("VELLUM_TLS_KEY").map(PathBuf::from).ok_or_else(|| {
                invalid("VELLUM_TLS_KEY", "required when VELLUM_TLS=files")
            })?,
        },
        "acme" => {
            let mut domains = list(var("VELLUM_ACME_DOMAINS"));
            if domains.is_empty() {
                domains.push(gateway.app_host());
                domains.extend(gateway.public_host());
                domains = domains
                    .into_iter()
                    .map(|d| d.split(':').next().unwrap_or_default().to_owned())
                    .collect();
            }
            Tls::Acme {
                domains,
                contact: list(var("VELLUM_ACME_CONTACT"))
                    .into_iter()
                    .map(|e| format!("mailto:{e}"))
                    .collect(),
                cache_dir: config.data_dir.join(".vellum").join("acme"),
                directory: var("VELLUM_ACME_DIRECTORY"),
            }
        }
        other => {
            return Err(invalid(
                "VELLUM_TLS",
                format!("`{other}` is not off, files or acme"),
            ));
        }
    };
    if tls != Tls::Off {
        gateway.https = true;
    }
    let redirect_http = var("VELLUM_HTTP_REDIRECT")
        .map(|a| {
            a.parse::<SocketAddr>()
                .map_err(|e| invalid("VELLUM_HTTP_REDIRECT", e))
        })
        .transpose()?;
    let (csrf_var, csrf_text) = match var("VELLUM_CSRF_KEY") {
        Some(key) => ("VELLUM_CSRF_KEY", Some(key)),
        None => match var("VELLUM_CSRF_KEY_FILE") {
            Some(path) => (
                "VELLUM_CSRF_KEY_FILE",
                Some(
                    std::fs::read_to_string(path.trim())
                        .map_err(|e| invalid("VELLUM_CSRF_KEY_FILE", e))?,
                ),
            ),
            None => ("VELLUM_CSRF_KEY", None),
        },
    };
    let csrf_key = csrf_text
        .map(|k| STANDARD.decode(k.trim()).map_err(|e| invalid(csrf_var, e)))
        .transpose()?;
    if csrf_key.as_ref().is_some_and(|k| k.len() < 32) {
        return Err(invalid(csrf_var, "must decode to at least 32 bytes"));
    }
    let limits = connection_limits(&tls, &var)?;
    let scanner = var("VELLUM_CLAMD_ADDRESS")
        .map(|a| a.trim().to_owned())
        .filter(|a| !a.is_empty())
        .map(|address| Clamd::new(address, CLAMD_TIMEOUT));
    Ok(Edge {
        gateway,
        serve: ServeConfig {
            bind: config.bind,
            tls,
            redirect_http,
            limits,
        },
        csrf_key,
        scanner,
    })
}

/// The connection limits. With TLS on, clients connect directly, so one
/// address may hold at most [`PER_IP_CONNECTIONS`]; behind a
/// TLS-terminating proxy every connection shares the proxy's address, so
/// there is no cap unless `VELLUM_MAX_CONNECTIONS_PER_IP` sets one.
fn connection_limits(
    tls: &Tls,
    var: impl Fn(&str) -> Option<String>,
) -> Result<vellum_api::server::ConnectionLimits, Error> {
    let per_ip_default = if *tls == Tls::Off {
        0
    } else {
        PER_IP_CONNECTIONS
    };
    Ok(vellum_api::server::ConnectionLimits {
        max_connections_per_ip: var("VELLUM_MAX_CONNECTIONS_PER_IP")
            .map(|v| {
                v.trim()
                    .parse::<usize>()
                    .map_err(|e| invalid("VELLUM_MAX_CONNECTIONS_PER_IP", e))
            })
            .transpose()?
            .unwrap_or(per_ip_default),
        // Large files on slow links get time for their size (D213).
        upload_prefixes: &[
            crate::uploads::PREFIX,
            crate::exports::IMPORT_PREFIX,
            crate::issue_import::PREFIX,
        ],
        ..vellum_api::server::ConnectionLimits::default()
    })
}

/// Reads the install's CSRF key, creating it (32 random bytes, owner-only)
/// on first start.
fn csrf_key_file(data_dir: &Path) -> Result<Vec<u8>, Error> {
    key_file(data_dir, "gateway-csrf.key")
}

/// The 32-byte key in `.vellum/keys/<name>`, created (readable by the owner
/// only) on first use.
///
/// # Errors
///
/// The key folder cannot be created or the file cannot be written.
pub fn key_file(data_dir: &Path, name: &str) -> Result<Vec<u8>, Error> {
    let dir = data_dir.join(".vellum").join("keys");
    let path = dir.join(name);
    let io = |e: std::io::Error| {
        Error::new(
            ErrorKind::Unavailable,
            format!("cannot use {}", path.display()),
        )
        .with_source(e)
    };
    if let Some(existing) = std::fs::read(&path).ok().filter(|k| k.len() >= 32)
    {
        return Ok(existing);
    }
    std::fs::create_dir_all(&dir).map_err(io)?;
    let mut key = vec![0u8; 32];
    getrandom::fill(&mut key).map_err(|e| {
        Error::new(ErrorKind::Internal, "the random source failed")
            .with_source(e)
    })?;
    let mut options = std::fs::OpenOptions::new();
    let _ = options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let _ = options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(io)?;
    std::io::Write::write_all(&mut file, &key).map_err(io)?;
    Ok(key)
}

/// The gateway `vellum serve` runs: every service, and the embedded web
/// app for every other path on the app host.
///
/// # Errors
///
/// An invalid operation or `GraphQL` schema.
pub fn app_gateway(
    services: &services::Services,
    config: GatewayConfig,
) -> Result<Gateway, Error> {
    let per_minute =
        u64::from(config.rate_per_second) * 60 + u64::from(config.rate_burst);
    let mut builder = GatewayBuilder::new(config, services::VERSION);
    // With an index, the per-caller allowance holds across replicas (D173).
    if let Some(counts) = services.rate_counts() {
        let limit = Arc::new(crate::limits::ClusterLimit::new(per_minute));
        limit.spawn_sync(counts.clone());
        builder = builder.shared_limit(limit);
    }
    if let Some(exempt) =
        crate::limits::forwarded_joins(|k| std::env::var(k).ok())
    {
        builder = builder.rate_exempt(exempt);
    }
    if let Some(site) = services.public_site() {
        builder = builder.public_site(site);
    }
    services
        .mount(builder)
        // Live editing is mounted at `/collab` with the services (#415);
        // the web app's fallback never answers there.
        .web_app(services.web_app(web::router(web::embedded())))
        .build()
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::terminate(),
        ) {
            Ok(mut signal) => {
                let _ = signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
}

/// Where sign-in records live: with a database URL, in the database so
/// every replica shares them (ADR 0037, D160), importing a single-binary
/// `identity.json` the first time; otherwise in that file.
fn identity_store(config: &Config) -> Result<Store, Error> {
    let Some(url) = &config.database_url else {
        return Store::open(&config.data_dir);
    };
    let store = Store::shared(IndexConfig::postgres(url.clone()))?;
    if store.import_file(&config.data_dir)? {
        tracing::info!(
            "imported identity.json into the database; the file is now \
                identity.json.imported"
        );
    }
    Ok(store)
}

/// Runs the server until a shutdown signal.
///
/// # Errors
///
/// Configuration, key-file, TLS or listener errors.
pub fn run(
    config_path: Option<&Path>,
    observability: Observability,
) -> Result<(), Error> {
    let config = Config::load(config_path).map_err(|e| {
        Error::new(ErrorKind::InvalidInput, e.to_string()).with_source(e)
    })?;
    let Edge {
        mut gateway,
        serve: serve_config,
        csrf_key,
        scanner,
    } = edge(&config, |k| std::env::var(k).ok())?;
    if let Some(scanner) = &scanner {
        tracing::info!(?scanner, "default upload virus scanner");
    }
    if csrf_key.is_none() && config.database_url.is_some() {
        tracing::warn!(
            "VELLUM_CSRF_KEY is not set: each replica makes its own key, so \
             forms fail when requests move between replicas; set it from a \
             shared secret"
        );
    }
    gateway.csrf_key = match csrf_key {
        Some(key) => key,
        None => csrf_key_file(&config.data_dir)?,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| {
            Error::new(ErrorKind::Internal, "cannot start the async runtime")
                .with_source(e)
        })?;
    let store = identity_store(&config)?;
    let app_origin = gateway.app_origin.clone();
    let sso = crate::identity::oidc_config(|k| std::env::var(k).ok())?;
    runtime.block_on(async move {
        let services = services::Services::open_with_store(
            &app_origin,
            Some(&config.data_dir),
            store,
            sso,
        )?
        .with_workspace(&config, |k| std::env::var(k).ok())
        .await?
        .with_scanner(scanner)
        .with_admin(&config, observability, |k| std::env::var(k).ok())
        .await?
        .with_assist(|k| std::env::var(k).ok())?;
        tracing::info!(services = ?services, "services ready");
        if !web::BUILT {
            tracing::warn!(
                "this binary was built without the web app; `/` \
                shows a placeholder"
            );
        }
        let gateway = app_gateway(&services, gateway)?;
        let databases = services.databases();
        tracing::info!(
            bind = %serve_config.bind,
            app = %host_of(&config.public_url.clone().unwrap_or_default()),
            "starting vellum"
        );
        // Live-editing sockets stay open until their rooms close, so save
        // and close the rooms before the server waits for connections.
        let stop = async move {
            shutdown_signal().await;
            services.shutdown().await;
        };
        let served = serve(gateway, serve_config, stop).await;
        close_databases(&databases).await;
        served
    })
}

/// How long shutdown waits for database connections still in use.
const DATABASE_CLOSE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(30);

/// Closes every workspace database while the runtime still runs, so no
/// connection is left closing on its own thread when the process exits.
/// An encrypted connection closing then races SQLCipher's exit handler.
async fn close_databases(databases: &[vellum_db::Index]) {
    let all = async {
        for database in databases {
            database.close().await;
        }
    };
    if tokio::time::timeout(DATABASE_CLOSE_TIMEOUT, all)
        .await
        .is_err()
    {
        tracing::warn!(
            "database connections still in use after \
                {DATABASE_CLOSE_TIMEOUT:?}; exiting anyway"
        );
    } else {
        tracing::info!(databases = databases.len(), "databases closed");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn edge_with(vars: &[(&str, &str)]) -> Result<Edge, Error> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let mut config = Config::default();
        config.public_url = Some("https://vellum.northstar.dev".into());
        edge(&config, |k| vars.get(k).cloned())
    }

    #[test]
    fn defaults_are_plain_http() -> Result<(), Error> {
        let e = edge_with(&[])?;
        assert_eq!(e.serve.tls, Tls::Off);
        assert_eq!(e.gateway.app_origin, "https://vellum.northstar.dev");
        assert!(e.gateway.public_origin.is_none());
        assert!(e.scanner.is_none());
        Ok(())
    }

    #[test]
    fn all_interfaces_bind_defaults_to_loopback_origin() {
        let origin = |bind: &str| default_origin(bind.parse().unwrap());
        assert_eq!(origin("0.0.0.0:7700"), "http://127.0.0.1:7700");
        assert_eq!(origin("[::]:7700"), "http://[::1]:7700");
        assert_eq!(origin("127.0.0.1:7700"), "http://127.0.0.1:7700");
        assert_eq!(origin("10.0.0.5:80"), "http://10.0.0.5:80");
    }

    #[test]
    fn per_ip_connection_cap_follows_tls() -> Result<(), Error> {
        assert_eq!(edge_with(&[])?.serve.limits.max_connections_per_ip, 0);
        let tls = edge_with(&[
            ("VELLUM_TLS", "files"),
            ("VELLUM_TLS_CERT", "/c.pem"),
            ("VELLUM_TLS_KEY", "/k.pem"),
        ])?;
        assert_eq!(tls.serve.limits.max_connections_per_ip, PER_IP_CONNECTIONS);
        let set = edge_with(&[("VELLUM_MAX_CONNECTIONS_PER_IP", "32")])?;
        assert_eq!(set.serve.limits.max_connections_per_ip, 32);
        assert!(
            edge_with(&[("VELLUM_MAX_CONNECTIONS_PER_IP", "many")]).is_err()
        );
        Ok(())
    }

    #[test]
    fn clamd_address_sets_the_default_scanner() -> Result<(), Error> {
        let e = edge_with(&[("VELLUM_CLAMD_ADDRESS", "clamav:3310")])?;
        assert!(format!("{:?}", e.scanner).contains("clamav:3310"));
        Ok(())
    }

    #[test]
    fn acme_defaults_to_the_app_and_pages_hosts() -> Result<(), Error> {
        let e = edge_with(&[
            ("VELLUM_TLS", "acme"),
            ("VELLUM_PUBLIC_PAGES_URL", "https://pages.northstar.dev/"),
            ("VELLUM_ACME_CONTACT", "ops@northstar.dev"),
        ])?;
        match e.serve.tls {
            Tls::Acme {
                domains, contact, ..
            } => {
                assert_eq!(
                    domains,
                    ["vellum.northstar.dev", "pages.northstar.dev"]
                );
                assert_eq!(contact, ["mailto:ops@northstar.dev"]);
            }
            other => {
                return Err(Error::new(
                    ErrorKind::Internal,
                    format!("{other:?}"),
                ));
            }
        }
        assert!(e.gateway.https);
        assert_eq!(
            e.gateway.public_origin.as_deref(),
            Some("https://pages.northstar.dev")
        );
        Ok(())
    }

    #[test]
    fn bad_values_name_the_variable() {
        for (vars, key) in [
            (vec![("VELLUM_TLS", "maybe")], "VELLUM_TLS"),
            (vec![("VELLUM_TLS", "files")], "VELLUM_TLS_CERT"),
            (
                vec![("VELLUM_HTTP_REDIRECT", "nope")],
                "VELLUM_HTTP_REDIRECT",
            ),
            (vec![("VELLUM_CSRF_KEY", "c2hvcnQ=")], "VELLUM_CSRF_KEY"),
            (
                vec![("VELLUM_CSRF_KEY_FILE", "/nonexistent/csrf")],
                "VELLUM_CSRF_KEY_FILE",
            ),
        ] {
            let err = edge_with(&vars)
                .err()
                .map(|e| e.message().to_owned())
                .unwrap_or_default();
            assert!(err.starts_with(key), "{err}");
        }
    }

    #[test]
    fn csrf_key_can_come_from_a_secret_file() -> Result<(), Error> {
        let dir = std::env::temp_dir()
            .join(format!("vellum-csrf-secret-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| invalid("dir", e))?;
        let path = dir.join("csrf");
        let key = STANDARD.encode([9u8; 32]);
        std::fs::write(&path, format!("{key}\n"))
            .map_err(|e| invalid("write", e))?;
        let path = path.display().to_string();
        let e = edge_with(&[("VELLUM_CSRF_KEY_FILE", path.as_str())])?;
        assert_eq!(e.csrf_key, Some(vec![9u8; 32]));
        // The variable wins over the file.
        let direct = STANDARD.encode([3u8; 32]);
        let e = edge_with(&[
            ("VELLUM_CSRF_KEY", direct.as_str()),
            ("VELLUM_CSRF_KEY_FILE", path.as_str()),
        ])?;
        assert_eq!(e.csrf_key, Some(vec![3u8; 32]));
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[test]
    fn csrf_key_file_is_created_once() -> Result<(), Error> {
        let dir = std::env::temp_dir()
            .join(format!("vellum-csrf-{}", std::process::id()));
        let first = csrf_key_file(&dir)?;
        let second = csrf_key_file(&dir)?;
        assert_eq!(first.len(), 32);
        assert_eq!(first, second);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
