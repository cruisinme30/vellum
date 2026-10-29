// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The composition root: every service the binary serves.
//!
//! Each service workstream adds its service to [`Services`] and
//! [`Services::mount`] (with any raw routers), so `vellum serve`, the
//! OpenAPI document and the `GraphQL` schema all see the same set.

use std::path::Path;
use std::sync::Arc;

use url::Url;
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::{Gateway, GatewayBuilder};
use vellum_contracts::Service;
use vellum_identity::{Identity, IdentityConfig, IdentityService, Store, http as identity_http};
use vellum_kernel::{Error, ErrorKind};

use crate::identity::{GuardedFetcher, IdentityAuthenticator};

/// The binary's version, shown in `/healthz` and the API documents.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Every service the binary runs.
#[derive(Debug, Clone)]
pub struct Services {
    identity: Arc<Identity>,
}

impl Services {
    /// Opens every service for the workspace at `public_url`, keeping
    /// state under `data_dir` (or in memory when `None`).
    ///
    /// # Errors
    ///
    /// Invalid settings or unreadable state.
    pub fn open(public_url: &str, data_dir: Option<&Path>) -> Result<Self, Error> {
        let url = Url::parse(public_url).map_err(|e| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("VELLUM_PUBLIC_URL is not a URL: {e}"),
            )
        })?;
        let mut config = IdentityConfig::new(url);
        config.data_dir = data_dir.map(Path::to_path_buf);
        let store = match data_dir {
            Some(dir) => Store::open(dir)?,
            None => Store::memory(),
        };
        let identity =
            Identity::new(config, store).with_metadata_fetcher(Arc::new(GuardedFetcher::new()?));
        Ok(Self {
            identity: Arc::new(identity),
        })
    }

    /// Every typed service, in registration order.
    pub fn all(&self) -> Vec<Arc<dyn Service>> {
        vec![Arc::new(IdentityService::new(Arc::clone(&self.identity)))]
    }

    /// Adds every service, the authenticator and the raw routers to a
    /// gateway builder.
    pub fn mount(&self, mut builder: GatewayBuilder) -> GatewayBuilder {
        for service in self.all() {
            builder = builder.service(service);
        }
        let identity = &self.identity;
        builder
            .authenticator(Arc::new(IdentityAuthenticator::new(Arc::clone(identity))))
            .mount("/auth", identity_http::browser_router(Arc::clone(identity)))
            .mount_cross_site("/oauth", identity_http::oauth_router(Arc::clone(identity)))
            .mount_cross_site(
                "/.well-known",
                identity_http::well_known_router(Arc::clone(identity)),
            )
    }
}

/// A gateway built only to describe the API (no listener, no storage).
pub fn contract_gateway() -> Result<Gateway, Error> {
    let services = Services::open("http://localhost", None)?;
    services
        .mount(GatewayBuilder::new(
            GatewayConfig::new("http://localhost", vec![0; 32]),
            VERSION,
        ))
        .build()
}
