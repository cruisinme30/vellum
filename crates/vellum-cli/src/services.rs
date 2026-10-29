// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The composition root: every service the binary serves.
//!
//! Each service workstream adds its service to [`Services`] and
//! [`Services::mount`] (with any raw routers), so `vellum serve`, the
//! OpenAPI document and the `GraphQL` schema all see the same set.
//! Identity always runs; the workspace services (drive, collections,
//! books, Work and My Work) join when an index is configured.

use std::path::Path;
use std::sync::Arc;

use url::Url;
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::{Gateway, GatewayBuilder};
use vellum_contracts::Service;
use vellum_identity::{Identity, IdentityConfig, IdentityService, Store, http as identity_http};
use vellum_kernel::{Config, Error, ErrorKind};
use vellum_tasks::service::TasksService;

use crate::identity::{GuardedFetcher, IdentityAuthenticator};
use crate::workspace;

/// The binary's version, shown in `/healthz` and the API documents.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Every service the binary runs.
#[derive(Clone)]
pub struct Services {
    identity: Arc<Identity>,
    /// The workspace services, when an index is configured.
    workspace: Vec<Arc<dyn Service>>,
    /// My Work, for its feed, CalDAV and capture routes.
    tasks: Option<Arc<TasksService>>,
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services")
            .field(
                "workspace",
                &self.workspace.iter().map(|s| s.name()).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
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
            workspace: Vec::new(),
            tasks: None,
        })
    }

    /// Adds the workspace services for `config` (see [`workspace`]).
    ///
    /// # Errors
    ///
    /// An unreachable database, a wrong key or an unreadable data folder.
    pub async fn with_workspace(
        mut self,
        config: &Config,
        var: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, Error> {
        let opened = workspace::open(config, &self.identity, var).await?;
        self.workspace = opened.services;
        self.tasks = opened.tasks;
        Ok(self)
    }

    /// Every typed service, in registration order.
    pub fn all(&self) -> Vec<Arc<dyn Service>> {
        let mut all: Vec<Arc<dyn Service>> =
            vec![Arc::new(IdentityService::new(Arc::clone(&self.identity)))];
        all.extend(self.workspace.iter().cloned());
        all
    }

    /// Adds every service, the authenticator and the raw routers to a
    /// gateway builder.
    pub fn mount(&self, mut builder: GatewayBuilder) -> GatewayBuilder {
        for service in self.all() {
            builder = builder.service(service);
        }
        let identity = &self.identity;
        if let Some(tasks) = &self.tasks {
            for (prefix, router) in crate::tasks_http::routers(tasks, identity) {
                builder = builder.mount_cross_site(prefix, router);
            }
        }
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

/// A gateway built only to describe the API: no listener, in-memory
/// permissions and an empty scratch folder that is removed afterwards.
pub fn contract_gateway() -> Result<Gateway, Error> {
    let mut services = Services::open("http://localhost", None)?;
    let scratch = std::env::temp_dir().join(format!("vellum-contracts-{}", std::process::id()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| {
            Error::new(ErrorKind::Internal, "cannot start the async runtime").with_source(e)
        })?;
    let described = runtime.block_on(workspace::describe(&scratch, &services.identity));
    let _ = std::fs::remove_dir_all(&scratch);
    services.workspace = described?;
    services
        .mount(GatewayBuilder::new(
            GatewayConfig::new("http://localhost", vec![0; 32]),
            VERSION,
        ))
        .build()
}
