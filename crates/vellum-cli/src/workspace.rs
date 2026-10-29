// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The workspace services: the drive, collections, books, Work, My Work and
//! live editing, over the files in the data folder, the index database and the
//! permission engine.
//!
//! `vellum serve` opens them over an index: Postgres through the kernel's
//! `database_url`, or the SQLite index in the data folder, encrypted with
//! `VELLUM_INDEX_KEY` (or plaintext when `VELLUM_INDEX_PLAINTEXT=1`). With
//! neither, the key is made on the first start and kept in
//! `.vellum/keys/index.key`, so `vellum serve` needs no settings.
//!
//! Workspace files live in the data folder, or in a shared S3 bucket when
//! `VELLUM_BLOB_URL` names one (`s3://bucket[/prefix]`, with
//! `VELLUM_S3_ENDPOINT`, `VELLUM_S3_REGION`, and the `AWS_*` keys or the
//! pod's workload identity). Several
//! replicas need the bucket and a Postgres index, so they all see one
//! workspace and a restarted pod loses nothing.
//!
//! At startup the first replica of the cluster rescans the files (see
//! [`crate::cluster`]). For a data folder, a file
//! watcher then keeps the index in step with edits made outside Vellum
//! (see [`crate::watcher`]); a bucket is only written through Vellum.
//! Writes are announced in the index's change feed, and each replica
//! follows it to keep its caches fresh (see [`crate::feed`]).
//! Search indexes the drive's pages and the collections' rows (see
//! [`crate::search`]). Every blob store call is metered
//! (`vellum_store_*`, see [`vellum_store::Metered`]).

use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures::StreamExt as _;
use vellum_authz::{Authz, AuthzStore, Id, MemoryStore, ShardedStore};
use vellum_books_service::BooksService;
use vellum_collections::CollectionsService;
use vellum_collections::service::IndexFeed;
use vellum_contracts::{BoxFuture, Operation, Readiness, Service};
use vellum_db::feed::Feed;
use vellum_db::{Index, IndexConfig, IndexKey, ShardRouter, ShardedIndex, ShardsConfig};
use vellum_drive::authz::{AuthzAccess as DriveAuthzAccess, TreeSync};
use vellum_drive::{Drive, DriveIndex, DriveService};
use vellum_identity::Identity;
use vellum_kernel::{Config, Error, ErrorKind, UserId};
use vellum_notify::endpoint::SafeTransport;
use vellum_notify::hooks::Webhooks;
use vellum_notify::mail::Mailer;
use vellum_notify::service::NotifyStore;
use vellum_store::remote::s3::{S3Config, S3Store};
use vellum_store::{BlobPath, BlobStore, LocalStore, Metered, StoreError};
use vellum_tasks::google::Google;
use vellum_tasks::service::{Config as TasksConfig, TasksService};
use vellum_work::{UserRef, WorkEvent};
use vellum_work_service::{Directory, EventSink, SystemClock, WorkService};

use crate::cluster::Replica;
use crate::collab::{Collab, LivePages};
use crate::feed::{AnnouncingStore, Caches};
use crate::google_http::{self, Endpoints, FileLinks, GoogleHttp};
use crate::identity::IdentityAuthenticator;
use crate::page_render::{Handles, PageRenderService};
use crate::reindex::{KEY_VAR, key_bytes};
use crate::tasks::{DriveTasks, Parts, TasksSettings};

/// The mail domain that receives My Work capture e-mail.
pub const MAIL_DOMAIN_VAR: &str = "VELLUM_MAIL_DOMAIN";

/// The Slack app's signing secret, for capture from Slack.
pub const SLACK_SECRET_VAR: &str = "VELLUM_SLACK_SIGNING_SECRET";

/// How often time blocks sync with Google Calendar.
const GOOGLE_SYNC_EVERY: Duration = Duration::from_mins(5);

/// Variable that opens the SQLite index without encryption.
pub const PLAINTEXT_VAR: &str = "VELLUM_INDEX_PLAINTEXT";

/// Variable naming the shared blob store; unset keeps files in the data
/// folder.
pub const BLOB_URL_VAR: &str = "VELLUM_BLOB_URL";

/// Folder the readiness probe lists. It is normally empty; any answer but
/// an error means the store is reachable. Listing, unlike a lookup, does
/// not log a missing file as an error on every probe.
const PROBE: &str = ".vellum/ready";

/// How often the space list behind permission checks is reloaded in full,
/// as a backstop to the change feed.
const SPACES_REFRESH: Duration = Duration::from_secs(30);

fn unavailable(what: &str, e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Unavailable, format!("{what}: {e}"))
}

/// The file under `.vellum/keys` that holds the index key when
/// `VELLUM_INDEX_KEY` is not set.
pub const KEY_FILE: &str = "index.key";

/// The SQLite index key: `VELLUM_INDEX_KEY`, then nothing in plaintext
/// mode, then the key kept beside the data in `.vellum/keys/index.key`,
/// made on the first start so `vellum serve` needs no settings.
///
/// # Errors
///
/// A malformed key, an index file left without its key (made with a key
/// that is not set now), or a key folder that cannot be written.
pub fn sqlite_key(
    config: &Config,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Option<[u8; 32]>, Error> {
    if let Some(hex) = var(KEY_VAR) {
        return key_bytes(Some(&hex))
            .map(Some)
            .map_err(|e| Error::new(ErrorKind::InvalidInput, e));
    }
    if var(PLAINTEXT_VAR).is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes")) {
        return Ok(None);
    }
    let vellum = config.data_dir.join(".vellum");
    if !vellum.join("keys").join(KEY_FILE).exists() && vellum.join("index.sqlite").exists() {
        // A new key cannot open the old index; never replace it silently.
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "the index in {} was made with a key that is not set now: set {KEY_VAR}, \
                 or restore .vellum/keys/{KEY_FILE}",
                vellum.display()
            ),
        ));
    }
    let bytes = crate::keys::key_file(&config.data_dir, KEY_FILE)?;
    bytes
        .get(..32)
        .and_then(|k| <[u8; 32]>::try_from(k).ok())
        .map(Some)
        .ok_or_else(|| Error::new(ErrorKind::Internal, "the index key file is too short"))
}

/// The index settings for `config`: Postgres when a database URL is set,
/// otherwise SQLite in the data folder, sealed with [`sqlite_key`].
///
/// # Errors
///
/// As [`sqlite_key`].
pub fn index_config(
    config: &Config,
    var: impl Fn(&str) -> Option<String>,
) -> Result<IndexConfig, Error> {
    if let Some(url) = &config.database_url {
        return Ok(IndexConfig::postgres(url.clone()));
    }
    let path = config.data_dir.join(".vellum").join("index.sqlite");
    let key = sqlite_key(config, &var)?;
    if key.is_some() && var(KEY_VAR).is_none() {
        tracing::warn!(
            "the index key is kept in the data folder (.vellum/keys/{KEY_FILE}); \
             copy it somewhere safe and set {KEY_VAR}"
        );
    }
    Ok(IndexConfig::sqlite(path, key.map(IndexKey::new)))
}

/// Where the workspace files live.
struct Files {
    store: Arc<dyn BlobStore>,
    /// A folder on this machine, which the file watcher can follow.
    local: bool,
}

/// The shared bucket `VELLUM_BLOB_URL` names, or `None` to keep files in
/// the data folder.
///
/// # Errors
///
/// A blob URL that is not `s3://`, or missing S3 settings.
pub fn bucket(var: impl Fn(&str) -> Option<String>) -> Result<Option<S3Config>, Error> {
    let Some(url) = var(BLOB_URL_VAR).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if !url.starts_with("s3://") {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "{BLOB_URL_VAR} must be an s3:// URL; leave it unset to keep files in the data folder"
            ),
        ));
    }
    S3Config::from_vars(var).map(Some).map_err(|e| {
        Error::new(
            ErrorKind::InvalidInput,
            format!("cannot use the blob store: {e}"),
        )
    })
}

/// Opens `bucket`, or the data folder when there is none.
///
/// # Errors
///
/// The S3 client cannot start, or the data folder cannot be opened.
async fn open_files(config: &Config, bucket: Option<S3Config>) -> Result<Files, Error> {
    let Some(bucket) = bucket else {
        let store = LocalStore::open(&config.data_dir)
            .await
            .map_err(|e| unavailable("cannot open the data folder", e))?;
        return Ok(Files {
            store: Arc::new(Metered::new(store)),
            local: true,
        });
    };
    let store = S3Store::new(bucket).map_err(|e| unavailable("cannot use the blob store", e))?;
    Ok(Files {
        store: Arc::new(Metered::new(store)),
        local: false,
    })
}

/// Warns when files are shared but the index is not, which only works
/// for one replica.
fn warn_if_unshared(local: bool, index: &IndexConfig) {
    if !local && matches!(index, IndexConfig::Sqlite { .. }) {
        tracing::warn!(
            "files are in a shared bucket but the index is this machine's SQLite file; \
             run one replica only, or set a database URL"
        );
    }
}

/// Reports whether the blob store answers, for `/readyz`. It has no
/// operations of its own.
///
/// A shared bucket that stops answering makes the replica degraded, not
/// unready (ADR 0044, D188): every replica shares the bucket, so failing
/// readiness would take them all out of the load balancer at once, and
/// the index-backed reads (search, lists, Work, My Work) would go down
/// with it. Calls that need the files fail on their own with
/// `unavailable` and a `Retry-After`. A data folder on this machine is
/// different: if it is gone, this replica cannot serve and another may.
#[derive(Debug)]
struct Storage {
    store: Arc<dyn BlobStore>,
    shared: bool,
}

impl Storage {
    fn new(store: Arc<dyn BlobStore>, shared: bool) -> Self {
        Self { store, shared }
    }
}

impl Service for Storage {
    fn name(&self) -> &'static str {
        "storage"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        Vec::new()
    }

    fn readiness(&self) -> BoxFuture<'_, Readiness> {
        Box::pin(async move { soften(probe(self.store.as_ref()).await, self.shared) })
    }
}

/// Turns an outage of a shared bucket into [`Readiness::Degraded`].
fn soften(readiness: Readiness, shared: bool) -> Readiness {
    match readiness {
        Readiness::NotReady(reason) if shared => {
            tracing::warn!(%reason, "the shared blob store is down: serving degraded");
            Readiness::Degraded(reason)
        }
        other => other,
    }
}

async fn probe(store: &dyn BlobStore) -> Readiness {
    let Ok(path) = BlobPath::new(PROBE) else {
        return Readiness::Ready;
    };
    match store.list(Some(&path)).next().await {
        None | Some(Ok(_) | Err(StoreError::NotFound(_))) => Readiness::Ready,
        Some(Err(e)) => Readiness::NotReady(format!(
            "the {} blob store is unreachable: {e}",
            store.backend()
        )),
    }
}

/// The spaces and their folders, newest first by folder depth, for the
/// synchronous path-to-space lookup the permission adapters need.
#[derive(Debug, Default)]
struct Spaces(RwLock<Vec<(BlobPath, Id)>>);

impl Spaces {
    fn replace(&self, mut spaces: Vec<(BlobPath, Id)>) {
        // Deepest folder first, so a nested space wins over its parent.
        spaces.sort_by_key(|(root, _)| std::cmp::Reverse(root.as_str().len()));
        if let Ok(mut current) = self.0.write() {
            *current = spaces;
        }
    }

    fn of(&self, path: &BlobPath) -> Option<Id> {
        self.0
            .read()
            .ok()?
            .iter()
            .find(|(root, _)| path.starts_with(root))
            .map(|(_, id)| *id)
    }
}

/// Follows the change feed into this replica's caches, and reloads the
/// space list in full every [`SPACES_REFRESH`] as a backstop.
fn keep_fresh(
    feed: Feed,
    me: String,
    cursor: i64,
    workspace: &Workspace,
    spaces: &Arc<Spaces>,
    drive_index: &Arc<dyn DriveIndex>,
) {
    let (reload, from) = (Arc::clone(spaces), Arc::clone(drive_index));
    crate::feed::follow(
        feed,
        me,
        cursor,
        Caches {
            collections: Arc::clone(&workspace.collections),
            books: Arc::clone(&workspace.books),
            spaces: Box::new(move || {
                let (reload, from) = (Arc::clone(&reload), Arc::clone(&from));
                Box::pin(async move {
                    match load_spaces(from.as_ref()).await {
                        Ok(list) => reload.replace(list),
                        Err(error) => tracing::warn!(%error, "space list not reloaded"),
                    }
                })
            }),
        },
    );
    let (refresh, drive_index) = (Arc::clone(spaces), Arc::clone(drive_index));
    drop(tokio::spawn(async move {
        let mut tick = tokio::time::interval(SPACES_REFRESH);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let _ = tick.tick().await;
            match load_spaces(drive_index.as_ref()).await {
                Ok(list) => refresh.replace(list),
                Err(error) => tracing::warn!(%error, "space list not refreshed"),
            }
        }
    }));
}

async fn load_spaces(drive: &dyn DriveIndex) -> Result<Vec<(BlobPath, Id)>, Error> {
    let spaces = drive
        .spaces()
        .await
        .map_err(|e| unavailable("cannot list spaces", e))?;
    Ok(spaces
        .into_iter()
        .map(|s| (s.root, Id(s.id.as_uuid().as_u128())))
        .collect())
}

/// How often accounts made since startup are checked for a My Drive.
const DRIVES_EVERY: Duration = Duration::from_secs(30);

/// Looks up handles for naming My Drives.
fn drive_handles(identity: &Arc<Identity>) -> vellum_drive::service::Handles {
    let identity = Arc::clone(identity);
    Arc::new(move |user| identity.user(user).ok().map(|u| u.handle))
}

/// Makes a My Drive for every active account that has none (D171): at
/// startup for existing installs, then on each refresh for accounts made
/// since. The drive API also makes one on a person's first visit.
async fn provision_drives(
    drive: &Drive,
    index: &dyn DriveIndex,
    identity: &Identity,
) -> Result<usize, Error> {
    let owners: std::collections::HashSet<UserId> = index
        .spaces()
        .await
        .map_err(|e| unavailable("cannot list spaces", e))?
        .into_iter()
        .filter(|s| s.kind == vellum_drive::SpaceKind::UserDrive)
        .filter_map(|s| s.owner)
        .collect();
    let mut made = 0;
    for user in identity.users() {
        if user.disabled || owners.contains(&user.id) {
            continue;
        }
        match drive.ensure_my_drive(user.id, Some(&user.handle)).await {
            Ok(_) => made += 1,
            Err(error) => tracing::warn!(%error, user = %user.id, "My Drive not created"),
        }
    }
    Ok(made)
}

/// Makes the missing My Drives now, then keeps checking for new accounts.
async fn keep_drives(
    drive: &Arc<Drive>,
    index: &Arc<dyn DriveIndex>,
    identity: &Arc<Identity>,
) -> Result<(), Error> {
    let made = provision_drives(drive, index.as_ref(), identity).await?;
    if made > 0 {
        tracing::info!(made, "made My Drives for accounts that had none");
    }
    let (drive, index, people) = (Arc::clone(drive), Arc::clone(index), Arc::clone(identity));
    drop(tokio::spawn(async move {
        let mut tick = tokio::time::interval(DRIVES_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let _ = tick.tick().await;
            if let Err(error) = provision_drives(&drive, index.as_ref(), &people).await {
                tracing::warn!(%error, "My Drives not checked");
            }
        }
    }));
    Ok(())
}

/// Work handles come from identity's unique handles.
#[derive(Debug)]
struct IdentityDirectory(Arc<Identity>);

impl Directory for IdentityDirectory {
    fn handle(&self, user: UserId) -> BoxFuture<'_, Option<UserRef>> {
        let found = self
            .0
            .user(user)
            .ok()
            .and_then(|u| UserRef::parse(&u.handle).ok());
        Box::pin(async move { found })
    }
}

/// Logs Work events; used when describing the API, where nothing is
/// notified.
#[derive(Debug)]
struct TracingSink;

impl EventSink for TracingSink {
    fn publish(&self, events: Vec<WorkEvent>) {
        self.publish_by(None, events);
    }

    fn publish_by(&self, actor: Option<&UserRef>, events: Vec<WorkEvent>) {
        for event in events {
            tracing::debug!(?event, actor = ?actor.map(ToString::to_string), "work event");
        }
    }
}

/// The opened services, plus the two the file watcher drives, collections
/// for the change feed and search, and Work, which the integration
/// receivers act through.
struct Workspace {
    services: Vec<Arc<dyn Service>>,
    drive: Arc<Drive>,
    collections: Arc<CollectionsService>,
    books: Arc<BooksService>,
    work: Arc<WorkService>,
    tasks: Arc<TasksService>,
    space_of: vellum_collections::service::SpaceOf,
    /// Private spaces, whose guard every replica keeps current.
    e2e: Arc<vellum_e2e::E2e>,
}

/// What `vellum serve` mounts for the workspace.
#[derive(Default)]
pub struct Opened {
    /// The typed services.
    pub services: Vec<Arc<dyn Service>>,
    /// The integration receivers, to mount cross-site at `/hooks`, when an
    /// integration secret is set (see [`crate::integrations`]).
    pub hooks: Option<axum::Router>,
    /// Live editing, when an index is configured.
    pub collab: Option<Collab>,
    /// This replica in the cluster's heartbeat table.
    pub replica: Option<Replica>,
    /// Rate-limit counts shared with the other replicas.
    pub rate_counts: Option<vellum_db::rate::SharedCounts>,
    /// My Work, which also serves the feed, CalDAV and capture routes.
    pub tasks: Option<Arc<TasksService>>,
    /// The drive index, which Admin → Usage counts pages and storage from.
    pub drive_index: Option<Arc<dyn DriveIndex>>,
    /// The drive, which also takes file uploads (see [`crate::uploads`]).
    pub drive: Option<Arc<Drive>>,
    /// Books, which export reads sections through (see [`crate::exports`]).
    pub books: Option<Arc<BooksService>>,
    /// Work, which issue imports create through (see [`crate::issue_import`]).
    pub work: Option<Arc<WorkService>>,
}

/// My Work's settings: where its secrets live and what it links to.
pub struct TasksSetup {
    /// The adapter's settings.
    pub settings: TasksSettings,
    /// The service's settings.
    pub config: TasksConfig,
    /// The Google `OAuth` client (ID and secret), for time-blocking.
    pub google: Option<(String, String)>,
}

impl TasksSetup {
    /// The settings for a server at `identity`'s public URL.
    pub fn new(
        identity: &Identity,
        data_dir: Option<&Path>,
        var: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let url = identity.config().public_url.clone();
        let mail_domain = var(MAIL_DOMAIN_VAR)
            .or_else(|| url.host_str().map(str::to_owned))
            .unwrap_or_else(|| "localhost".to_owned());
        Self {
            settings: TasksSettings {
                dir: data_dir.map(|d| d.join(".vellum").join("tasks")),
                time_zone: None,
            },
            config: TasksConfig {
                base_url: url.as_str().trim_end_matches('/').to_owned(),
                mail_domain,
                slack_signing_secret: var(SLACK_SECRET_VAR)
                    .filter(|s| !s.is_empty())
                    .map(String::into_bytes),
            },
            google: var(google_http::CLIENT_ID_VAR)
                .filter(|s| !s.is_empty())
                .zip(var(google_http::CLIENT_SECRET_VAR).filter(|s| !s.is_empty())),
        }
    }
}

/// Where the services announce what changed.
struct Feeds {
    /// Work's events, for notifications.
    events: Arc<dyn EventSink>,
    /// Collection row changes, for search.
    rows: Option<Arc<dyn IndexFeed>>,
    /// Page text saved through the API, for live editing rooms.
    pages: Option<Arc<dyn vellum_drive::PageWrites>>,
    /// Which spaces are private: the e2e service changes it, the store's
    /// guard enforces it.
    private: vellum_e2e::PrivateRoots,
}

/// Builds the services over `store`, `authz` and the drive's `index`.
async fn build<S: AuthzStore + TreeSync + 'static>(
    store: Arc<dyn BlobStore>,
    authz: &Arc<Authz<S>>,
    index: Arc<dyn DriveIndex>,
    spaces: &Arc<Spaces>,
    identity: &Arc<Identity>,
    feeds: Feeds,
    setup: TasksSetup,
) -> Result<Workspace, Error> {
    let drive_access: Arc<dyn vellum_drive::Access> = Arc::new(
        vellum_drive::authz::AuthzAccess::new(Arc::clone(authz), Arc::clone(&index)),
    );
    let drive = Arc::new(Drive::new(
        Arc::clone(&store),
        Arc::clone(&index),
        Arc::clone(&drive_access),
    ));
    let e2e = Arc::new(crate::e2e::service(&store, &index, drive_access).with_roots(feeds.private));
    let lookup = Arc::clone(spaces);
    let space_of: vellum_collections::service::SpaceOf =
        Arc::new(move |path: &BlobPath| lookup.of(path));
    let mut collections = CollectionsService::new(
        Arc::clone(&store),
        Arc::new(vellum_collections::service::AuthzAccess::new(
            Arc::clone(authz),
            Arc::clone(&space_of),
        )),
    );
    if let Some(feed) = feeds.rows {
        collections = collections.with_index_feed(feed);
    }
    let collections = Arc::new(collections);
    let books = Arc::new(BooksService::new(
        Arc::clone(&store),
        Arc::new(vellum_books_service::AuthzAccess::new(
            Arc::clone(authz),
            Arc::clone(&space_of),
        )),
    ));
    let directory = Arc::new(IdentityDirectory(Arc::clone(identity)));
    let (work, report) = WorkService::open(
        Arc::clone(&store),
        Arc::clone(&directory) as Arc<dyn Directory>,
        feeds.events,
        Arc::new(SystemClock),
    )
    .await?;
    let work = Arc::new(work);
    if report.skipped > 0 {
        tracing::warn!(
            skipped = report.skipped,
            "some Work files could not be read"
        );
    }
    let adapter = DriveTasks::open(
        Parts {
            drive: Arc::clone(&drive),
            store,
            index,
            authz: Arc::clone(authz),
            identity: Arc::clone(identity),
            work: Arc::clone(&work),
        },
        &setup.settings,
    )?;
    let mut tasks = TasksService::new(Arc::new(adapter), setup.config);
    if let Some((id, secret)) = setup.google {
        let api = GoogleHttp::new(Endpoints::google()?, id.clone(), secret)?;
        let links = FileLinks::new(setup.settings.dir.map(|d| d.join("google")));
        tasks = tasks.with_google(Google::new(Arc::new(api), Arc::new(links), id));
    }
    let tasks = Arc::new(tasks);
    let handles: Handles = Arc::new(move |user| {
        let directory = Arc::clone(&directory);
        Box::pin(async move { directory.handle(user).await })
    });
    let pages = PageRenderService::new(
        Arc::clone(&drive),
        Arc::clone(&work),
        handles,
        PageRenderService::figures_from_env()?,
    );
    let mut drive_service =
        DriveService::new(Arc::clone(&drive)).with_handles(drive_handles(identity));
    if let Some(writes) = feeds.pages {
        drive_service = drive_service.with_page_writes(writes);
    }
    Ok(Workspace {
        services: vec![
            Arc::new(drive_service),
            Arc::clone(&collections) as Arc<dyn Service>,
            Arc::clone(&books) as Arc<dyn Service>,
            Arc::clone(&work) as Arc<dyn Service>,
            Arc::clone(&tasks) as Arc<dyn Service>,
            Arc::new(pages),
            Arc::clone(&e2e) as Arc<dyn Service>,
        ],
        drive,
        collections,
        books,
        work,
        tasks,
        space_of,
        e2e,
    })
}

/// The key the search index is derived from; `None` leaves it unsealed
/// (Postgres and plaintext mode).
fn search_key(
    config: &Config,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Option<[u8; 32]>, Error> {
    if config.database_url.is_some() {
        return Ok(None);
    }
    sqlite_key(config, var)
}

/// Joins the cluster's heartbeat table, and opens the rate-limit counts
/// shared with the other replicas. Also says whether another replica was
/// already alive.
async fn join_cluster(
    index: &Index,
    me: &str,
    var: impl Fn(&str) -> Option<String>,
) -> Result<(Replica, bool, vellum_db::rate::SharedCounts), Error> {
    let (replica, others_alive) = Replica::join(index, me, var)
        .await
        .map_err(|e| unavailable("cannot join the cluster", e))?;
    Ok((
        replica,
        others_alive,
        vellum_db::rate::SharedCounts::new(index.clone()),
    ))
}

/// The drive and permission stores, routing each space to its shard:
/// one of those `VELLUM_SHARDS` lists, or `index` alone (ADR 0040, D262).
/// Also returns every shard's database, home first.
async fn open_shards(
    index: &Index,
    var: impl Fn(&str) -> Option<String>,
) -> Result<(Vec<Index>, Arc<Authz<ShardedStore>>, Arc<dyn DriveIndex>), Error> {
    let sharded =
        match ShardsConfig::from_env(var).map_err(|e| unavailable("bad shard settings", e))? {
            Some(config) => {
                let sharded = ShardedIndex::open(&config)
                    .await
                    .map_err(|e| unavailable("cannot open the shards", e))?;
                tracing::info!(
                    shards = sharded.shards().len(),
                    "spaces are spread over shards"
                );
                sharded
            }
            None => ShardedIndex::single(index.clone()),
        };
    let router = ShardRouter::open(sharded)
        .await
        .map_err(|e| unavailable("cannot read the space-to-shard map", e))?;
    let router = Arc::new(router);
    Ok((
        router.indexes().cloned().collect(),
        Arc::new(Authz::new(ShardedStore::new(Arc::clone(&router)))),
        Arc::new(vellum_drive::sharded::ShardedIndex::new(router)),
    ))
}

/// Rescans the workspace files once per cluster (a replica that starts
/// while others run finds the index already current), then starts the
/// cluster's upkeep (D163).
async fn rescan_and_maintain(
    replica: &Replica,
    others_alive: bool,
    drive: &Arc<Drive>,
    index: &Index,
    shards: &[Index],
) {
    let drive = Arc::clone(drive);
    replica
        .rescan_once(others_alive, async move {
            match drive.scan().await {
                Ok(report) => {
                    tracing::info!(?report, "drive index rebuilt from the workspace files");
                }
                Err(error) => tracing::warn!(%error, "drive rescan failed; the index may be stale"),
            }
        })
        .await;
    replica.maintain(index, shards);
}

/// My Work's cluster jobs: Google Calendar sync every
/// [`GOOGLE_SYNC_EVERY`], and reminders and digests, each on one replica.
fn task_jobs(
    replica: &Replica,
    tasks: &Arc<TasksService>,
    notify: &Arc<NotifyStore>,
    data_dir: &Path,
) -> Result<(), Error> {
    crate::reminders::start(replica, tasks, notify, data_dir)?;
    let timer = Arc::clone(tasks);
    replica.every("tasks.google_sync", GOOGLE_SYNC_EVERY, move || {
        let timer = Arc::clone(&timer);
        async move {
            if let Err(error) = timer.google_sync_all().await {
                tracing::warn!(%error, "Google Calendar sync skipped");
            }
        }
    });
    Ok(())
}

/// Starts the notifier on this replica and its delivery timer on one
/// replica of the cluster.
fn notify_on_one_replica(
    replica: &Replica,
    queued: tokio::sync::mpsc::Receiver<vellum_work::WorkEvent>,
    work: &Arc<WorkService>,
    (identity, notify, webhooks): (
        &Arc<Identity>,
        &Arc<NotifyStore>,
        &Arc<Webhooks<SafeTransport>>,
    ),
    var: impl Fn(&str) -> Option<String>,
) -> Result<(), Error> {
    let delivery = delivery(identity, notify, webhooks, var)?;
    let ticker = crate::notifications::start(
        queued,
        Arc::clone(work),
        Arc::clone(identity),
        Arc::clone(notify),
        delivery,
    )?;
    replica.every("notify.deliver", crate::notifications::TICK, move || {
        let ticker = ticker.clone();
        async move { ticker.run().await }
    });
    Ok(())
}

/// Webhooks and email for the notifier. With a mail server configured,
/// email is turned on for every dispatcher over the notify store.
fn delivery(
    identity: &Arc<Identity>,
    notify: &Arc<NotifyStore>,
    webhooks: &Arc<Webhooks<SafeTransport>>,
    var: impl Fn(&str) -> Option<String>,
) -> Result<crate::notifications::Delivery, Error> {
    let mail = crate::mail::setup(var, identity.config().public_url.as_str())?.map(
        |(outbound, config)| {
            notify.enable_email();
            Arc::new(Mailer::new(Arc::clone(notify), outbound, config))
        },
    );
    Ok(crate::notifications::Delivery {
        webhooks: Arc::clone(webhooks),
        mail,
        people: Arc::new(crate::mail::People::new(Arc::clone(identity))),
    })
}

/// Opens the workspace services for `config`, or returns none (with a
/// warning) when no index is configured.
///
/// # Errors
///
/// An unreachable database, a wrong key, an unreadable data folder or
/// invalid integration settings.
#[allow(clippy::too_many_lines)] // the composition root names every service once
pub async fn open(
    config: &Config,
    identity: &Arc<Identity>,
    notify: &Arc<NotifyStore>,
    webhooks: &Arc<Webhooks<SafeTransport>>,
    app_origin: &str,
    var: impl Fn(&str) -> Option<String> + Send + Sync,
) -> Result<Opened, Error> {
    let setup = TasksSetup::new(identity, Some(&config.data_dir), &var);
    let bucket = bucket(&var)?;
    let key = search_key(config, &var)?;
    let index_config = index_config(config, &var)?;
    let index = Index::open(&index_config)
        .await
        .map_err(|e| unavailable("cannot open the index", e))?;
    let Files {
        store: files,
        local,
    } = open_files(config, bucket).await?;
    warn_if_unshared(local, &index_config);
    tracing::info!(backend = files.backend(), "workspace files opened");
    // Follow the feed from before anything is loaded, so no change made
    // meanwhile is missed.
    let feed = Feed::new(index.clone());
    let me = crate::feed::origin(&var);
    let cursor = feed
        .head()
        .await
        .map_err(|e| unavailable("cannot read the change feed", e))?;
    let (replica, others_alive, rate_counts) = join_cluster(&index, &me, &var).await?;
    let (store, roots) =
        crate::e2e::guarded(Arc::new(AnnouncingStore::new(files, feed.clone(), &me)));
    let (shards, authz, drive_index) = open_shards(&index, &var).await?;
    let spaces = Arc::new(Spaces::default());
    let (events, queued) = crate::notifications::channel();
    let (rows, row_changes) = crate::search::row_feed();
    let live_pages = Arc::new(LivePages::default());
    let mut workspace = build(
        Arc::clone(&store),
        &authz,
        Arc::clone(&drive_index),
        &spaces,
        identity,
        Feeds {
            events,
            rows: Some(rows),
            pages: Some(Arc::clone(&live_pages) as Arc<dyn vellum_drive::PageWrites>),
            private: roots,
        },
        setup,
    )
    .await?;
    notify_on_one_replica(
        &replica,
        queued,
        &workspace.work,
        (identity, notify, webhooks),
        &var,
    )?;
    // The watcher starts before the rescan, so nothing edited in between
    // is missed; whatever it sees twice is applied idempotently.
    if local {
        crate::watcher::start(&config.data_dir, &store, &workspace.drive, &workspace.books).await?;
    }
    rescan_and_maintain(&replica, others_alive, &workspace.drive, &index, &shards).await;
    // After the rescan, so every private space's folder is in the index.
    crate::e2e::keep_roots_fresh(Arc::clone(&workspace.e2e)).await;
    let hooks = crate::integrations::hooks(&store, &workspace.work, &var).await?;
    let collab = Collab::open(
        Arc::clone(&workspace.drive),
        Arc::new(DriveAuthzAccess::new(
            Arc::clone(&authz),
            Arc::clone(&drive_index),
        )),
        index.clone(),
        &config.data_dir,
        app_origin,
        Arc::new(IdentityAuthenticator::new(Arc::clone(identity))),
    )?;
    keep_drives(&workspace.drive, &drive_index, identity).await?;
    live_pages.attach(&collab);
    spaces.replace(load_spaces(drive_index.as_ref()).await?);
    let search = crate::search::start(crate::search::Parts {
        data_dir: config.data_dir.clone(),
        key,
        index,
        shards,
        store: Arc::clone(&store),
        authz,
        collections: Arc::clone(&workspace.collections),
        space_of: Arc::clone(&workspace.space_of),
        row_changes,
        holder: me.clone(),
    })
    .await?;
    workspace.services.push(Arc::new(search));
    keep_fresh(feed, me, cursor, &workspace, &spaces, &drive_index);
    task_jobs(&replica, &workspace.tasks, notify, &config.data_dir)?;
    let mut services = workspace.services;
    services.push(Arc::new(Storage::new(store, !local)));
    Ok(Opened {
        services,
        hooks,
        collab: Some(collab),
        replica: Some(replica),
        rate_counts: Some(rate_counts),
        tasks: Some(workspace.tasks),
        drive_index: Some(drive_index),
        drive: Some(workspace.drive),
        books: Some(workspace.books),
        work: Some(workspace.work),
    })
}

/// The services over an empty scratch folder and in-memory permissions,
/// for printing the API contracts.
///
/// # Errors
///
/// The scratch folder cannot be created.
pub async fn describe(
    scratch: &Path,
    identity: &Arc<Identity>,
) -> Result<Vec<Arc<dyn Service>>, Error> {
    let store: Arc<dyn BlobStore> = Arc::new(
        LocalStore::open(scratch)
            .await
            .map_err(|e| unavailable("cannot open a scratch folder", e))?,
    );
    let authz = Arc::new(Authz::new(MemoryStore::new()));
    let index = Arc::new(vellum_drive::memory::MemoryIndex::new());
    let setup = TasksSetup::new(identity, None, |_| None);
    let mut workspace = build(
        store,
        &authz,
        index,
        &Arc::new(Spaces::default()),
        identity,
        Feeds {
            events: Arc::new(TracingSink),
            rows: None,
            pages: None,
            private: vellum_e2e::PrivateRoots::default(),
        },
        setup,
    )
    .await?;
    workspace
        .services
        .push(Arc::new(crate::search::describe(authz)?));
    Ok(workspace.services)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn path(p: &str) -> BlobPath {
        BlobPath::new(p).unwrap()
    }

    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[tokio::test]
    async fn files_stay_local_unless_a_bucket_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.data_dir = dir.path().to_path_buf();
        let none = bucket(lookup(&[(BLOB_URL_VAR, "")])).unwrap();
        let local = open_files(&config, none).await.unwrap();
        assert!(local.local);
        assert_eq!(local.store.backend(), "local");
        assert_eq!(probe(local.store.as_ref()).await, Readiness::Ready);

        let named = bucket(lookup(&[
            (BLOB_URL_VAR, "s3://vellum/ws"),
            ("VELLUM_S3_ENDPOINT", "http://127.0.0.1:9"),
            ("AWS_ACCESS_KEY_ID", "id"),
            ("AWS_SECRET_ACCESS_KEY", "secret"),
        ]))
        .unwrap();
        let shared = open_files(&config, named).await.unwrap();
        assert!(!shared.local);
        assert_eq!(shared.store.backend(), "s3");
    }

    #[test]
    fn a_bad_blob_url_or_missing_keys_stop_startup() {
        for vars in [
            &[(BLOB_URL_VAR, "file:///data")][..],
            // A custom endpoint with no keys and no workload identity.
            &[
                (BLOB_URL_VAR, "s3://vellum"),
                ("VELLUM_S3_ENDPOINT", "http://127.0.0.1:9000"),
            ][..],
        ] {
            let error = bucket(lookup(vars)).err().unwrap();
            assert_eq!(error.kind(), ErrorKind::InvalidInput);
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[tokio::test]
    async fn an_unreachable_bucket_is_degraded() {
        // Port 9 (discard) refuses connections on a test machine.
        let store = S3Config::from_vars(lookup(&[
            (BLOB_URL_VAR, "s3://vellum"),
            ("VELLUM_S3_ENDPOINT", "http://127.0.0.1:9"),
            ("AWS_ACCESS_KEY_ID", "id"),
            ("AWS_SECRET_ACCESS_KEY", "secret"),
        ]))
        .and_then(S3Store::new)
        .unwrap();
        let down = probe(&store).await;
        assert!(matches!(&down, Readiness::NotReady(reason) if reason.contains("s3")));
        // Shared by every replica: degraded, so the pods stay in rotation.
        assert!(
            matches!(soften(down.clone(), true), Readiness::Degraded(reason) if reason.contains("s3"))
        );
        // A data folder on this machine: unready, so traffic goes elsewhere.
        assert!(matches!(soften(down, false), Readiness::NotReady(_)));
        assert_eq!(soften(Readiness::Ready, true), Readiness::Ready);
    }

    #[test]
    fn the_deepest_space_folder_wins() {
        let spaces = Spaces::default();
        spaces.replace(vec![
            (path("spaces/eng"), Id(1)),
            (path("spaces/eng/platform"), Id(2)),
        ]);
        assert_eq!(spaces.of(&path("spaces/eng/platform/a.md")), Some(Id(2)));
        assert_eq!(spaces.of(&path("spaces/eng/b.md")), Some(Id(1)));
        assert_eq!(spaces.of(&path("other/c.md")), None);
    }

    #[test]
    fn index_uses_a_key_plaintext_a_database_or_a_kept_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::default();
        config.data_dir = dir.path().to_path_buf();
        let vars = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect()
        };
        let plain = vars(&[(PLAINTEXT_VAR, "1")]);
        assert!(matches!(
            index_config(&config, |k| plain.get(k).cloned()).unwrap(),
            IndexConfig::Sqlite { key: None, .. }
        ));
        let keyed = vars(&[(KEY_VAR, &"ab".repeat(32))]);
        assert!(matches!(
            index_config(&config, |k| keyed.get(k).cloned()).unwrap(),
            IndexConfig::Sqlite { key: Some(_), .. }
        ));
        let bad = vars(&[(KEY_VAR, "nope")]);
        assert!(index_config(&config, |k| bad.get(k).cloned()).is_err());
        assert!(!dir.path().join(".vellum/keys").join(KEY_FILE).exists());

        // With no settings, a key is made once and kept beside the data.
        let none = vars(&[]);
        let first = sqlite_key(&config, |k| none.get(k).cloned()).unwrap();
        assert!(first.is_some());
        assert_eq!(
            sqlite_key(&config, |k| none.get(k).cloned()).unwrap(),
            first
        );
        assert!(matches!(
            index_config(&config, |k| none.get(k).cloned()).unwrap(),
            IndexConfig::Sqlite { key: Some(_), .. }
        ));

        // An index whose key is gone is refused, not silently re-keyed.
        std::fs::remove_file(dir.path().join(".vellum/keys").join(KEY_FILE)).unwrap();
        std::fs::write(dir.path().join(".vellum/index.sqlite"), b"sealed").unwrap();
        let error = sqlite_key(&config, |k| none.get(k).cloned()).unwrap_err();
        assert!(error.to_string().contains(KEY_VAR), "{error}");
        assert!(!dir.path().join(".vellum/keys").join(KEY_FILE).exists());
    }
}
