// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The composition root: every service the binary serves.
//!
//! Each service workstream adds its service to [`Services`] and
//! [`Services::mount`] (with any raw routers), so `vellum serve`, the
//! OpenAPI document and the `GraphQL` schema all see the same set.
//! Identity and notifications always run; the workspace services (drive,
//! collections, books, Work and My Work) join when an index is configured,
//! and the admin console when `vellum serve` opens it over the data folder.

use std::path::Path;
use std::sync::Arc;

use url::Url;
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::{Gateway, GatewayBuilder};
use vellum_contracts::Service;
use vellum_identity::{
    Identity, IdentityConfig, IdentityService, Sso, SsoConfig, Store, http as identity_http,
};
use vellum_kernel::{Config, Error, ErrorKind};
use vellum_notify::service::{NotifyService, NotifyStore};
use vellum_tasks::service::TasksService;

use crate::collab::Collab;
use crate::identity::{GuardedFetcher, GuardedSsoHttp, IdentityAuthenticator};
use crate::observability::{self, Observability};
use crate::workspace;

/// The binary's version, shown in `/healthz` and the API documents.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Every service the binary runs.
#[derive(Clone)]
pub struct Services {
    identity: Arc<Identity>,
    /// Push subscriptions and notification preferences.
    notify: Arc<NotifyService>,
    /// The app's origin, which live-editing connections must come from.
    origin: String,
    /// Collections, books and Work, when an index is configured.
    workspace: Vec<Arc<dyn Service>>,
    /// The integration receivers, when an integration secret is set.
    hooks: Option<axum::Router>,
    /// Live editing, when an index is configured.
    collab: Option<Collab>,
    /// This replica's place in the cluster, when an index is configured.
    replica: Option<crate::cluster::Replica>,
    /// Rate-limit counts shared with the other replicas.
    rate_counts: Option<vellum_db::rate::SharedCounts>,
    /// My Work, for its feed, CalDAV and capture routes.
    tasks: Option<Arc<TasksService>>,
    /// Share links on the public origin, when an index is configured.
    public: Option<Arc<dyn vellum_api::public::PublicSite>>,
    /// The admin console (requests, routes, audit, settings, backups).
    admin: Option<Arc<dyn Service>>,
    /// Where the gateway records per-route metrics.
    observability: Observability,
}

impl std::fmt::Debug for Services {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Services")
            .field(
                "workspace",
                &self.workspace.iter().map(|s| s.name()).collect::<Vec<_>>(),
            )
            .field("hooks", &self.hooks.is_some())
            .field("admin", &self.admin.is_some())
            .finish_non_exhaustive()
    }
}

impl Services {
    /// Opens every service for the workspace at `public_url`, keeping
    /// state under `data_dir` (or in memory when `None`), with single sign-on
    /// through `sso` when set.
    ///
    /// # Errors
    ///
    /// Invalid settings or unreadable state.
    pub fn open(
        public_url: &str,
        data_dir: Option<&Path>,
        sso: Option<SsoConfig>,
    ) -> Result<Self, Error> {
        let store = match data_dir {
            Some(dir) => Store::open(dir)?,
            None => Store::memory(),
        };
        Self::open_with_store(public_url, data_dir, store, sso)
    }

    /// As [`Services::open`], with the identity records in `store` (the
    /// shared database store when several replicas run).
    ///
    /// # Errors
    ///
    /// Invalid settings.
    pub fn open_with_store(
        public_url: &str,
        data_dir: Option<&Path>,
        store: Store,
        sso: Option<SsoConfig>,
    ) -> Result<Self, Error> {
        let url = Url::parse(public_url).map_err(|e| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("VELLUM_PUBLIC_URL is not a URL: {e}"),
            )
        })?;
        let subject = url.to_string();
        let mut config = IdentityConfig::new(url.clone());
        config.data_dir = data_dir.map(Path::to_path_buf);
        let mut identity =
            Identity::new(config, store).with_metadata_fetcher(Arc::new(GuardedFetcher::new()?));
        if let Some(sso) = sso {
            let http = Arc::new(GuardedSsoHttp::new(&sso.issuer)?);
            tracing::info!(issuer = %sso.issuer, "single sign-on is on");
            identity = identity.with_sso(Sso::new(sso, http));
        }
        let notify = match data_dir {
            Some(dir) => NotifyStore::open(dir, &subject)?,
            None => NotifyStore::memory(&subject)?,
        };
        Ok(Self {
            identity: Arc::new(identity),
            notify: Arc::new(NotifyService::new(Arc::new(notify))),
            origin: url.origin().ascii_serialization(),
            workspace: Vec::new(),
            hooks: None,
            collab: None,
            replica: None,
            rate_counts: None,
            tasks: None,
            public: None,
            admin: None,
            observability: Observability::new(),
        })
    }

    /// Adds the admin console over `config`'s data folder, reading and
    /// writing the same trace buffer and route metrics as `observability`.
    /// Scheduled backups, drills and usage reports start running.
    ///
    /// # Errors
    ///
    /// A malformed key, an unreadable folder or a broken audit chain.
    pub async fn with_admin(
        mut self,
        config: &Config,
        observability: Observability,
        var: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, Error> {
        let (service, facts) = observability::open(config, &observability, var).await?;
        observability::spawn_ticks(Arc::clone(&service), Arc::clone(&facts));
        self.admin = Some(observability::api(service, facts));
        self.observability = observability;
        Ok(self)
    }

    /// Adds the workspace services for `config` (see [`workspace`]).
    ///
    /// # Errors
    ///
    /// An unreachable database, a wrong key or an unreadable data folder.
    pub async fn with_workspace(
        mut self,
        config: &Config,
        var: impl Fn(&str) -> Option<String> + Send + Sync,
    ) -> Result<Self, Error> {
        let opened = workspace::open(
            config,
            &self.identity,
            self.notify.store(),
            &self.origin,
            var,
        )
        .await?;
        self.workspace = opened.services;
        self.hooks = opened.hooks;
        self.collab = opened.collab;
        self.replica = opened.replica;
        self.rate_counts = opened.rate_counts;
        self.tasks = opened.tasks;
        self.public = opened.public;
        Ok(self)
    }

    /// Share links on the public origin, when an index is configured.
    pub fn public_site(&self) -> Option<Arc<dyn vellum_api::public::PublicSite>> {
        self.public.clone()
    }

    /// Rate-limit counts shared with the other replicas, when an index is
    /// configured.
    pub const fn rate_counts(&self) -> Option<&vellum_db::rate::SharedCounts> {
        self.rate_counts.as_ref()
    }

    /// The live-editing endpoint, mounted at `/collab` by [`Self::mount`]:
    /// one WebSocket per page at `/collab/{space_id}/{doc_id}`. Without a
    /// workspace index it answers 503.
    pub fn collab(&self) -> axum::Router {
        self.collab
            .as_ref()
            .map_or_else(crate::collab::unavailable_router, Collab::router)
    }

    /// Saves every page open for live editing and closes its connections,
    /// then leaves the cluster's heartbeat table.
    /// Call it after the server stops accepting requests.
    pub async fn shutdown(&self) {
        if let Some(collab) = &self.collab {
            collab.shutdown().await;
        }
        if let Some(replica) = &self.replica {
            replica.leave().await;
        }
    }

    /// Every typed service, in registration order.
    pub fn all(&self) -> Vec<Arc<dyn Service>> {
        let mut all: Vec<Arc<dyn Service>> = vec![
            Arc::new(IdentityService::new(Arc::clone(&self.identity))),
            Arc::clone(&self.notify) as Arc<dyn Service>,
        ];
        all.extend(self.workspace.iter().cloned());
        all.extend(self.admin.iter().cloned());
        all
    }

    /// Adds every service, the authenticator and the raw routers to a
    /// gateway builder.
    pub fn mount(&self, mut builder: GatewayBuilder) -> GatewayBuilder {
        for service in self.all() {
            builder = builder.service(service);
        }
        let identity = &self.identity;
        if let Some(hooks) = &self.hooks {
            // Providers sign each delivery and send no CSRF token.
            builder = builder.mount_cross_site("/hooks", hooks.clone());
        }
        if let Some(tasks) = &self.tasks {
            for (prefix, router) in crate::tasks_http::routers(tasks, identity) {
                builder = builder.mount_cross_site(prefix, router);
            }
        }
        builder
            .route_metrics(self.observability.metrics.clone())
            .authenticator(Arc::new(IdentityAuthenticator::new(Arc::clone(identity))))
            .mount("/auth", identity_http::browser_router(Arc::clone(identity)))
            .mount_cross_site("/oauth", identity_http::oauth_router(Arc::clone(identity)))
            .mount_cross_site(
                "/.well-known",
                identity_http::well_known_router(Arc::clone(identity)),
            )
            .mount("/collab", self.collab())
    }
}

/// A gateway built only to describe the API: no listener, in-memory
/// permissions and an empty scratch folder that is removed afterwards.
pub fn contract_gateway() -> Result<Gateway, Error> {
    let mut services = Services::open("http://localhost", None, None)?;
    let scratch = std::env::temp_dir().join(format!("vellum-contracts-{}", std::process::id()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| {
            Error::new(ErrorKind::Internal, "cannot start the async runtime").with_source(e)
        })?;
    let described = runtime.block_on(workspace::describe(&scratch, &services.identity));
    let admin = observability::describe(&scratch);
    let _ = std::fs::remove_dir_all(&scratch);
    services.workspace = described?;
    services.admin = Some(admin?);
    services
        .mount(GatewayBuilder::new(
            GatewayConfig::new("http://localhost", vec![0; 32]),
            VERSION,
        ))
        .build()
}
