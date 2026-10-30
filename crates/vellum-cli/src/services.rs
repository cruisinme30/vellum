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
use crate::{exports, issue_import, uploads, workspace};

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
    /// Group membership, for SCIM, when an index is configured.
    group_members: Option<Arc<dyn vellum_identity::GroupMembers>>,
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
    /// Books, for their public sites at `/published`.
    books: Option<Arc<vellum_books_service::BooksService>>,
    /// The admin audit log, which records the admin actions every other
    /// service serves (D243).
    admin_audit: Option<Arc<dyn vellum_admin::AuditLog>>,
    /// Where the gateway records per-route metrics.
    observability: Observability,
    /// The drive index, which Admin → Usage counts pages and storage from.
    drive_index: Option<Arc<dyn vellum_drive::DriveIndex>>,
    /// The MCP endpoint, when an index is configured (ADR 0019, D217).
    mcp: Option<Arc<vellum_mcp::http::McpHttp>>,
    /// The workspace MCP tools run against, for other in-process callers
    /// that want the same permission-checked reads and drafts.
    mcp_workspace: Option<Arc<dyn vellum_mcp::ports::Workspace>>,
    /// Where MCP calls are audited.
    audit: Arc<crate::mcp_serve::AuditTrail>,
    /// The drive, for file uploads, when an index is configured.
    drive: Option<Arc<vellum_drive::Drive>>,
    /// The virus scanner every upload goes through, when one is set.
    scanner: Option<Arc<Clamd>>,
    /// Secret scanning on save, following the admin settings (D296).
    secrets: Arc<crate::secrets::SecretGuard>,
    /// Search's admin side, when an index is configured (D300).
    search: Option<Arc<crate::search_admin::SearchStatus>>,
    /// Bring-your-own-model features, when the MCP workspace is wired.
    assist: Option<Arc<dyn Service>>,
    /// The public URL, for links out of an export.
    public_url: String,
    /// The same workspace, for wiring the export routes into MCP (D265).
    mcp_adapter: Option<Arc<crate::mcp_workspace::ServeWorkspace>>,
    /// Work, which issue imports create through.
    work: Option<Arc<vellum_work_service::WorkService>>,
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
            .field("assist", &self.assist.is_some())
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
        let shared = store.is_shared();
        let mut identity =
            Identity::new(config, store).with_metadata_fetcher(Arc::new(GuardedFetcher::new()?));
        if let Some(key) = identity_key(data_dir, shared, |k| std::env::var(k).ok())? {
            identity = identity.with_sealing_key(&key)?;
        }
        identity = identity.with_sso_http(Arc::new(|issuer: &Url| {
            let http: Arc<dyn vellum_identity::SsoHttp> = Arc::new(GuardedSsoHttp::new(issuer)?);
            Ok(http)
        }));
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
            group_members: None,
            collab: None,
            replica: None,
            rate_counts: None,
            tasks: None,
            public: None,
            admin: None,
            books: None,
            admin_audit: None,
            observability: Observability::new(),
            drive_index: None,
            mcp: None,
            mcp_workspace: None,
            audit: Arc::default(),
            drive: None,
            scanner: None,
            secrets: Arc::default(),
            search: None,
            assist: None,
            public_url: public_url.trim_end_matches('/').to_owned(),
            mcp_adapter: None,
            work: None,
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
        let (sampler, facts) = observability::counted(config, counts);
        observability::spawn_sampling(sampler);
        self.audit.attach(service.audit_log());
        self.secrets.attach(Arc::clone(&service));
        if let Some(search) = &self.search {
            search.follow(Arc::clone(&service));
        }
        observability::spawn_ticks(Arc::clone(&service), Arc::clone(&facts));
        self.admin_audit = Some(service.audit_log());
        self.admin = Some(observability::api(service, facts));
        self.observability = observability;
        Ok(self)
    }

    /// Adds the bring-your-own-model API over the MCP workspace adapter
    /// (see [`crate::assist`]). Without that adapter it stays off.
    ///
    /// # Errors
    ///
    /// A malformed `VELLUM_AI_ENDPOINTS`.
    pub fn with_assist(mut self, var: impl Fn(&str) -> Option<String>) -> Result<Self, Error> {
        if let Some(workspace) = &self.mcp_workspace {
            self.assist = crate::assist::service(Arc::clone(workspace), var)?;
        }
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
            &self.secrets,
            var,
        )
        .await?;
        self.workspace = opened.services;
        self.hooks = opened.hooks;
        self.group_members = opened.group_members;
        self.collab = opened.collab;
        self.replica = opened.replica;
        self.rate_counts = opened.rate_counts;
        self.tasks = opened.tasks;
        self.drive_index = opened.drive_index;
        self.search = opened.search;
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
        self.books = opened.books;
        self.drive = opened.drive;
        self.public = opened.public;
        self.work = opened.work;
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

    /// Share links on the public origin, when an index is configured.
    pub fn public_site(&self) -> Option<Arc<dyn vellum_api::public::PublicSite>> {
        self.public.clone()
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

    /// The issue import route, mounted at [`issue_import::PREFIX`] by
    /// [`Self::mount`]. Without a workspace index it answers 503.
    pub fn issue_import(&self) -> axum::Router {
        self.work
            .as_ref()
            .map_or_else(issue_import::unavailable_router, |work| {
                issue_import::router(
                    Arc::clone(work),
                    Arc::clone(&self.identity),
                    self.scanner.clone(),
                    UploadPolicy::default(),
                )
            })
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

    /// The help books, mounted at `/guide` by [`Self::mount`]. Diagrams
    /// draw with the same settings as pages; a malformed diagram setting
    /// (already refused for pages) falls back to the built-in defaults.
    pub fn help() -> axum::Router {
        let figures = crate::page_render::PageRenderService::figures_from_env()
            .map_err(|e| e.to_string())
            .or_else(|_| {
                vellum_render::figure::Figures::new(vellum_render::figure::FigureConfig::default())
            });
        let figures = match figures {
            Ok(figures) => Some(figures),
            Err(error) => {
                tracing::warn!(%error, "help books: diagrams are unavailable");
                None
            }
        };
        figures.map_or_else(axum::Router::new, crate::help_http::router)
    }

    /// The web app, answering `?format=md` on page URLs with Markdown
    /// once the workspace is open (R-AI-6, D295).
    pub fn web_app(&self, web_app: axum::Router) -> axum::Router {
        match &self.mcp_workspace {
            Some(workspace) => crate::site_http::markdown_pages(
                web_app,
                Arc::clone(workspace),
                Arc::new(IdentityAuthenticator::new(Arc::clone(&self.identity))),
            ),
            None => web_app,
        }
    }

    /// Leaves the cluster's heartbeat table, so the other replicas take
    /// over this one's live-editing rooms at once, then saves every page
    /// open for live editing and closes its connections.
    /// Call it after the server stops accepting requests.
    pub async fn shutdown(&self) {
        // Stop both heartbeats before saving the rooms, so neither puts
        // this replica back in the list while its editors move away.
        if let Some(collab) = &self.collab {
            collab.leave_cluster().await;
        }
        if let Some(replica) = &self.replica {
            replica.leave().await;
        }
        if let Some(collab) = &self.collab {
            collab.shutdown().await;
        }
    }

    /// Every typed service, in registration order. With the admin
    /// console, every other service's admin actions are audited (D243).
    pub fn all(&self) -> Vec<Arc<dyn Service>> {
        let mut all: Vec<Arc<dyn Service>> = vec![
            Arc::new(IdentityService::new(Arc::clone(&self.identity))),
            Arc::clone(&self.notify) as Arc<dyn Service>,
            Arc::new(WebhooksService::new(Arc::clone(&self.webhooks))),
        ];
        all.push(Arc::new(crate::secrets::SecretsService::new(Arc::clone(
            &self.secrets,
        ))));
        all.extend(self.workspace.iter().cloned());
        if let Some(log) = &self.admin_audit {
            all = all
                .into_iter()
                .map(|service| vellum_admin::audited::audited(service, Arc::clone(log)))
                .collect();
        }
        all.extend(self.admin.iter().cloned());
        if let Some(search) = &self.search {
            all.push(Arc::new(crate::search_admin::SearchAdminService::new(
                Arc::clone(search),
            )));
        }
        all.extend(self.assist.iter().cloned());
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
        if let Some(books) = &self.books {
            builder = builder.mount("/published", crate::published::router(Arc::clone(books)));
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
        if let Some(workspace) = &self.mcp_workspace {
            // `llms.txt` and the Markdown sitemap (R-AI-6, D295).
            for (path, router) in crate::site_http::mounts(workspace) {
                builder = builder.mount(path, router);
            }
        }
        builder
            .route_metrics(self.observability.metrics.clone())
            .authenticator(Arc::new(IdentityAuthenticator::new(Arc::clone(identity))))
            .mount("/auth", identity_http::browser_router(Arc::clone(identity)))
            .mount_cross_site("/oauth", identity_http::oauth_router(Arc::clone(identity)))
            .mount_cross_site("/saml", identity_http::saml_router(Arc::clone(identity)))
            // Providers provision with their own bearer token (D294).
            .mount_cross_site(
                vellum_identity::scim::SCIM_PREFIX,
                vellum_identity::scim::router(Arc::clone(identity), self.group_members.clone()),
            )
            .mount_cross_site("/.well-known", well_known)
            .mount("/collab", self.collab())
            // The help books, for the app's `?` buttons (ADR 0024, D256).
            .mount(crate::help_http::PREFIX, Self::help())
            .mount_uploads(uploads::PREFIX, self.uploads(), upload_body_limit())
            .mount(exports::EXPORT_PREFIX, export)
            .mount_uploads(exports::IMPORT_PREFIX, import, upload_body_limit())
            .mount_uploads(
                issue_import::PREFIX,
                self.issue_import(),
                upload_body_limit(),
            )
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

/// Overrides the key that seals authenticator secrets and identity
/// provider secrets (64 hex characters). Replicas must share it.
pub const IDENTITY_KEY_VAR: &str = "VELLUM_IDENTITY_KEY";

/// The identity key (ADR 0069, D290): `VELLUM_IDENTITY_KEY`, or
/// `.vellum/keys/identity.key` in the data folder, created on first start.
/// Without a data folder, `None`: identity uses a key that lasts until
/// the process ends.
///
/// # Errors
///
/// A malformed variable or an unreadable key file.
pub fn identity_key(
    data_dir: Option<&Path>,
    shared: bool,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Option<[u8; 32]>, Error> {
    if let Some(hex) = var(IDENTITY_KEY_VAR).filter(|v| !v.trim().is_empty()) {
        return crate::reindex::parse_hex_key(IDENTITY_KEY_VAR, hex.trim())
            .map(Some)
            .map_err(|why| Error::new(ErrorKind::InvalidInput, why));
    }
    let Some(dir) = data_dir else {
        return Ok(None);
    };
    if shared {
        tracing::warn!(
            "VELLUM_IDENTITY_KEY is not set: each replica makes its own identity key, so \
             authenticator codes fail on replicas other than the one that set them up; \
             set it from a shared secret"
        );
    }
    let bytes = crate::keys::key_file(dir, "identity.key")?;
    let key: [u8; 32] = bytes
        .get(..32)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| Error::new(ErrorKind::Internal, "the identity key is too short"))?;
    Ok(Some(key))
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
    services.search = Some(crate::search::describe_status()?);
    services.assist = crate::assist::service(
        Arc::new(vellum_mcp::memory::MemoryWorkspace::new("http://localhost")),
        |_| None,
    )?;
    services
        .mount(GatewayBuilder::new(
            GatewayConfig::new("http://localhost", vec![0; 32]),
            VERSION,
        ))
        .build()
}
