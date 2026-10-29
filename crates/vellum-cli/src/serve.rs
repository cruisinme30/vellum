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
//! | `VELLUM_EMBED_ORIGINS` | Comma-separated origins allowed in sandboxed embeds |

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use vellum_api::config::{GatewayConfig, host_of};
use vellum_api::gateway::GatewayBuilder;
use vellum_api::server::{ServeConfig, Tls, serve};
use vellum_kernel::{Config, Error, ErrorKind};

use crate::services;

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
}

/// Builds the edge settings from `config` and a variable lookup.
///
/// # Errors
///
/// `invalid_input` naming the variable that is wrong.
pub fn edge(config: &Config, var: impl Fn(&str) -> Option<String>) -> Result<Edge, Error> {
    let app_origin = config
        .public_url
        .clone()
        .unwrap_or_else(|| format!("http://{}", config.bind));
    let mut gateway = GatewayConfig::new(app_origin, Vec::new());
    gateway.public_origin =
        var("VELLUM_PUBLIC_PAGES_URL").map(|u| u.trim_end_matches('/').to_owned());
    gateway.embed_origins = list(var("VELLUM_EMBED_ORIGINS"));
    let tls = match var("VELLUM_TLS").as_deref().unwrap_or("off") {
        "off" => Tls::Off,
        "files" => Tls::Files {
            cert: var("VELLUM_TLS_CERT")
                .map(PathBuf::from)
                .ok_or_else(|| invalid("VELLUM_TLS_CERT", "required when VELLUM_TLS=files"))?,
            key: var("VELLUM_TLS_KEY")
                .map(PathBuf::from)
                .ok_or_else(|| invalid("VELLUM_TLS_KEY", "required when VELLUM_TLS=files"))?,
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
    let csrf_key = var("VELLUM_CSRF_KEY")
        .map(|k| {
            STANDARD
                .decode(k.trim())
                .map_err(|e| invalid("VELLUM_CSRF_KEY", e))
        })
        .transpose()?;
    if csrf_key.as_ref().is_some_and(|k| k.len() < 32) {
        return Err(invalid(
            "VELLUM_CSRF_KEY",
            "must decode to at least 32 bytes",
        ));
    }
    Ok(Edge {
        gateway,
        serve: ServeConfig {
            bind: config.bind,
            tls,
            redirect_http,
        },
        csrf_key,
    })
}

/// Reads the install's CSRF key, creating it (32 random bytes, owner-only)
/// on first start.
fn csrf_key_file(data_dir: &Path) -> Result<Vec<u8>, Error> {
    let dir = data_dir.join(".vellum").join("keys");
    let path = dir.join("gateway-csrf.key");
    let io = |e: std::io::Error| {
        Error::new(
            ErrorKind::Unavailable,
            format!("cannot use {}", path.display()),
        )
        .with_source(e)
    };
    if let Some(existing) = std::fs::read(&path).ok().filter(|k| k.len() >= 32) {
        return Ok(existing);
    }
    std::fs::create_dir_all(&dir).map_err(io)?;
    let mut key = vec![0u8; 32];
    getrandom::fill(&mut key)
        .map_err(|e| Error::new(ErrorKind::Internal, "the random source failed").with_source(e))?;
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

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
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

/// Runs the server until a shutdown signal.
///
/// # Errors
///
/// Configuration, key-file, TLS or listener errors.
pub fn run(config_path: Option<&Path>) -> Result<(), Error> {
    let config = Config::load(config_path)
        .map_err(|e| Error::new(ErrorKind::InvalidInput, e.to_string()).with_source(e))?;
    let Edge {
        mut gateway,
        serve: serve_config,
        csrf_key,
    } = edge(&config, |k| std::env::var(k).ok())?;
    gateway.csrf_key = match csrf_key {
        Some(key) => key,
        None => csrf_key_file(&config.data_dir)?,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| {
            Error::new(ErrorKind::Internal, "cannot start the async runtime").with_source(e)
        })?;
    let app_origin = gateway.app_origin.clone();
    runtime.block_on(async move {
        let services = services::Services::open(&app_origin, Some(&config.data_dir))?;
        let gateway = services
            .mount(GatewayBuilder::new(gateway, services::VERSION))
            .build()?;
        tracing::info!(bind = %serve_config.bind, app = %host_of(&config.public_url.clone().unwrap_or_default()), "starting vellum");
        serve(gateway, serve_config, shutdown_signal()).await
    })
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
                assert_eq!(domains, ["vellum.northstar.dev", "pages.northstar.dev"]);
                assert_eq!(contact, ["mailto:ops@northstar.dev"]);
            }
            other => return Err(Error::new(ErrorKind::Internal, format!("{other:?}"))),
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
        ] {
            let err = edge_with(&vars)
                .err()
                .map(|e| e.message().to_owned())
                .unwrap_or_default();
            assert!(err.starts_with(key), "{err}");
        }
    }

    #[test]
    fn csrf_key_file_is_created_once() -> Result<(), Error> {
        let dir = std::env::temp_dir().join(format!("vellum-csrf-{}", std::process::id()));
        let first = csrf_key_file(&dir)?;
        let second = csrf_key_file(&dir)?;
        assert_eq!(first.len(), 32);
        assert_eq!(first, second);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
