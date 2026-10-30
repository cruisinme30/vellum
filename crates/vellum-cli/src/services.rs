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
use vellum_notify::endpoint::{self, SafeTransport};
use vellum_notify::hooks::{HookStore, Webhooks, WebhooksService};
use vellum_notify::service::{NotifyService, NotifyStore};
use vellum_security::upload::{Clamd, UploadPolicy};
use vellum_tasks::service::TasksService;

use crate::collab::Collab;
use crate::identity::{GuardedFetcher, GuardedSsoHttp, IdentityAuthenticator};
use crate::observability::{self, Observability};
use crate::{exports, uploads, workspace};

/// The binary's version, shown in `/healthz` and the API documents.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Every service the binary runs.
#[derive(Clone)]
pub struct Services {
    identity: Arc<Identity>,
    /// Push subscriptions and notification preferences.
    notify: Arc<NotifyService>,
    /// Webhook subscriptions and their delivery queue.
    webhooks: Arc<Webhooks<SafeTransport>>,
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
    /// The admin console (requests, routes, audit, settings, backups).
    admin: Option<Arc<dyn Service>>,
    /// Where the gateway records per-route metrics.
    observability: Observability,
    /// The MCP endpoint, when an index is configured (ADR 0019, D217).
    mcp: Option<Arc<vellum_mcp::http::McpHttp>>,
    /// The workspace MCP tools run against, for other in-process callers
    /// that want the same permission-checked reads and drafts.
    mcp_workspace: Option<Arc<dyn vellum_mcp::ports::Workspace>>,
    /// Where MCP calls are audited.
    audit: Arc<crate::mcp_serve::AuditTrail>,
    /// The same workspace, for wiring the export routes into MCP (D265).
    mcp_adapter: Option<Arc<crate::mcp_workspace::ServeWorkspace>>,
    /// The drive index, which Admin → Usage counts pages and storage from.
    drive_index: Option<Arc<dyn vellum_drive::DriveIndex>>,
    /// The drive, for file uploads, when an index is configured.
    drive: Option<Arc<vellum_drive::Drive>>,
    /// The virus scanner every upload goes through, when one is set.
    scanner: Option<Arc<Clamd>>,
    /// Books, which book exports read through.
    books: Option<Arc<vellum_books_service::BooksService>>,
    /// The public URL, for links out of an export.
    public_url: String,
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
            .field("mcp", &self.mcp.is_some())
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
        let hooks = match data_dir {
            Some(dir) => HookStore::open(dir, endpoint::policy(&[]))?,
            None => HookStore::memory(endpoint::policy(&[])),
        };
        let transport = SafeTransport::new(endpoint::policy(&[])).map_err(|e| {
            Error::new(
                ErrorKind::Internal,
                format!("cannot build the webhook client: {e}"),
            )
        })?;
        Ok(Self {
            identity: Arc::new(identity),
            notify: Arc::new(NotifyService::new(Arc::new(notify))),
            webhooks: Arc::new(Webhooks::new(Arc::new(hooks), transport)),
            origin: url.origin().ascii_serialization(),
            workspace: Vec::new(),
            hooks: None,
            collab: None,
            replica: None,
            rate_counts: None,
            tasks: None,
            admin: None,
            observability: Observability::new(),
            mcp: None,
            mcp_workspace: None,
            audit: Arc::default(),
            mcp_adapter: None,
            drive_index: None,
            drive: None,
            scanner: None,
            books: None,
            public_url: public_url.trim_end_matches('/').to_owned(),
        })
    }

    /// Adds the admin console over `config`'s data folder, reading and
    /// writing the same trace buffer and route metrics as `observability`.
    /// Scheduled backups, drills and usage reports start running, and the
    /// usage counts (Admin → Usage, `vellum.usage.*`) are taken every few
    /// minutes. Call it after [`Self::with_workspace`], so pages and storage
    /// are counted too.
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
        let counts = Arc::new(observability::WorkspaceCounts {
            identity: Arc::clone(&self.identity),
            drive: self.drive_index.clone(),
        });
        let (service, _) = observability::open(config, &observability, var).await?;
        self.audit.attach(service.audit_log());
        let (sampler, facts) = observability::counted(config, counts);
        observability::spawn_sampling(sampler);
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
            &self.webhooks,
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
        if let Some((workspace, events)) = opened.mcp {
            let shared = self.rate_counts.as_ref().map(|counts| {
                let per_minute = vellum_mcp::ratelimit::RateLimit::default().per_minute;
                let limit = Arc::new(crate::limits::ClusterLimit::new(u64::from(per_minute)));
                limit.spawn_sync(counts.clone());
                limit as Arc<dyn vellum_mcp::ratelimit::SharedLimit>
            });
            self.mcp_adapter = Some(Arc::clone(&workspace));
            let workspace = workspace as Arc<dyn vellum_mcp::ports::Workspace>;
            let mcp = crate::mcp_serve::endpoint(
                Arc::clone(&workspace),
                &self.identity,
                &self.origin,
                Arc::clone(&self.audit),
                shared,
            );
            events.attach(mcp.server.notifier());
            self.mcp = Some(mcp);
            self.mcp_workspace = Some(workspace);
        }
        self.drive_index = opened.drive_index;
        self.drive = opened.drive;
        self.books = opened.books;
        Ok(self)
    }

    /// Scans every upload with `scanner` (`VELLUM_CLAMD_ADDRESS`).
    #[must_use]
    pub fn with_scanner(mut self, scanner: Option<Clamd>) -> Self {
        self.scanner = scanner.map(Arc::new);
        self
    }

    /// The upload routes, mounted at [`uploads::PREFIX`] by [`Self::mount`].
    /// Without a workspace index they answer 503.
    pub fn uploads(&self) -> axum::Router {
        self.drive
            .as_ref()
            .map_or_else(uploads::unavailable_router, |drive| {
                uploads::router(
                    Arc::clone(drive),
                    self.scanner.clone(),
                    UploadPolicy::default(),
                )
            })
    }

    /// The export and import routers, mounted at [`exports::EXPORT_PREFIX`]
    /// and [`exports::IMPORT_PREFIX`] by [`Self::mount`]. Without a
    /// workspace index they answer 503.
    pub fn transfer(&self) -> (axum::Router, axum::Router) {
        let Some(drive) = &self.drive else {
            return (exports::unavailable_router(), exports::unavailable_router());
        };
        let figures = crate::page_render::PageRenderService::figures_from_env()
            .inspect_err(|e| tracing::warn!(error = %e, "exports draw no figures"))
            .ok()
            .map(Arc::new);
        let transfer = Arc::new(exports::Transfer::new(
            Arc::clone(drive),
            self.books.clone(),
            figures,
            self.scanner.clone(),
            UploadPolicy::default(),
            Some(self.public_url.clone()),
        ));
        if let Some(adapter) = &self.mcp_adapter {
            adapter.set_exporter(mcp_exporter(&transfer));
        }
        (transfer.export_router(), transfer.import_router())
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
            Arc::new(WebhooksService::new(Arc::clone(&self.webhooks))),
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
        let (export, import) = self.transfer();
        if let Some(hooks) = &self.hooks {
            // Providers sign each delivery and send no CSRF token.
            builder = builder.mount_cross_site("/hooks", hooks.clone());
        }
        if let Some(tasks) = &self.tasks {
            for (prefix, router) in crate::tasks_http::routers(tasks, identity) {
                builder = builder.mount_cross_site(prefix, router);
            }
        }
        // Mail apps post one-click unsubscribes with no session (RFC 8058).
        builder = builder.mount_cross_site(
            "/unsubscribe",
            crate::mail::router(Arc::clone(self.notify.store())),
        );
        let mut well_known = identity_http::well_known_router(Arc::clone(identity));
        if let Some(mcp) = &self.mcp {
            // Bearer tokens only, from clients on other origins: no CSRF.
            builder = builder
                .mount_cross_site("/mcp", vellum_mcp::http::endpoint_router(Arc::clone(mcp)));
            well_known = well_known.merge(vellum_mcp::http::metadata_router(Arc::clone(mcp)));
        }
        builder
            .route_metrics(self.observability.metrics.clone())
            .authenticator(Arc::new(IdentityAuthenticator::new(Arc::clone(identity))))
            .mount("/auth", identity_http::browser_router(Arc::clone(identity)))
            .mount_cross_site("/oauth", identity_http::oauth_router(Arc::clone(identity)))
            .mount_cross_site("/.well-known", well_known)
            .mount("/collab", self.collab())
            .mount_uploads(uploads::PREFIX, self.uploads(), upload_body_limit())
            .mount(exports::EXPORT_PREFIX, export)
            .mount_uploads(exports::IMPORT_PREFIX, import, upload_body_limit())
    }
}

/// The MCP `export` tool's hook into the export service (ADR 0065, D265).
fn mcp_exporter<S: Send + Sync + 'static>(
    transfer: &Arc<exports::Transfer<S>>,
) -> crate::mcp_workspace::Exporter {
    use crate::mcp_workspace::{ExportFile, ExportWhat};
    let transfer = Arc::clone(transfer);
    Arc::new(move |principal, what, format| {
        let transfer = Arc::clone(&transfer);
        Box::pin(async move {
            let item = match what {
                ExportWhat::Page(id) => exports::ExportItem::Page(id),
                ExportWhat::Book { id, section } => {
                    let book = id.parse().map_err(|_| {
                        vellum_kernel::Error::new(
                            vellum_kernel::ErrorKind::NotFound,
                            "book not found",
                        )
                    })?;
                    let scope = section.map_or(
                        vellum_export::book::BookScope::Whole,
                        vellum_export::book::BookScope::Subtree,
                    );
                    exports::ExportItem::Book(book, scope)
                }
            };
            let file = transfer
                .export(
                    &principal,
                    vellum_kernel::RequestId::generate(),
                    item,
                    &format,
                    vellum_export::pdf::Paper::A4,
                )
                .await?;
            Ok(ExportFile {
                file_name: file.file_name,
                mime: file.mime.to_owned(),
                bytes: file.bytes,
            })
        })
    })
}

/// The largest upload body: the upload policy's file limit.
fn upload_body_limit() -> usize {
    usize::try_from(UploadPolicy::default().max_bytes).unwrap_or(usize::MAX)
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
