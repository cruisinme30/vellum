// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The workspace services: the drive, collections, books, Work and My
//! Work, over the files in the data folder, the index database and the
//! permission engine.
//!
//! `vellum serve` opens them when an index is configured: Postgres through
//! the kernel's `database_url`, or the SQLite index in the data folder,
//! encrypted with `VELLUM_INDEX_KEY` (or plaintext when
//! `VELLUM_INDEX_PLAINTEXT=1`). Without one the server still starts, with
//! sign-in only, and says why.
//!
//! At startup the drive rescans the data folder, and a file watcher then
//! keeps the index in step with edits made outside Vellum (see
//! [`crate::watcher`]).

use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use vellum_authz::{Authz, AuthzStore, Id, MemoryStore, SqlStore};
use vellum_books_service::BooksService;
use vellum_collections::CollectionsService;
use vellum_contracts::{BoxFuture, Service};
use vellum_db::{Index, IndexConfig};
use vellum_drive::authz::TreeSync;
use vellum_drive::sql::SqlIndex;
use vellum_drive::{Drive, DriveIndex, DriveService};
use vellum_identity::Identity;
use vellum_kernel::{Config, Error, ErrorKind, UserId};
use vellum_store::{BlobPath, BlobStore, LocalStore};
use vellum_tasks::google::Google;
use vellum_tasks::service::{Config as TasksConfig, TasksService};
use vellum_work::{UserRef, WorkEvent};
use vellum_work_service::{Directory, EventSink, SystemClock, WorkService};

use crate::google_http::{self, Endpoints, FileLinks, GoogleHttp};
use crate::reindex::{KEY_VAR, index_key};
use crate::tasks::{DriveTasks, Parts, TasksSettings};

/// The mail domain that receives My Work capture e-mail.
pub const MAIL_DOMAIN_VAR: &str = "VELLUM_MAIL_DOMAIN";

/// The Slack app's signing secret, for capture from Slack.
pub const SLACK_SECRET_VAR: &str = "VELLUM_SLACK_SIGNING_SECRET";

/// How often time blocks sync with Google Calendar.
const GOOGLE_SYNC_EVERY: Duration = Duration::from_mins(5);

/// Variable that opens the SQLite index without encryption.
pub const PLAINTEXT_VAR: &str = "VELLUM_INDEX_PLAINTEXT";

/// How often the space list behind permission checks is reloaded.
const SPACES_REFRESH: Duration = Duration::from_secs(30);

fn unavailable(what: &str, e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Unavailable, format!("{what}: {e}"))
}

/// The index settings for `config`, or `None` with the reason when no
/// index is configured.
///
/// # Errors
///
/// A key that is set but malformed.
pub fn index_config(
    config: &Config,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Result<IndexConfig, String>, Error> {
    if let Some(url) = &config.database_url {
        return Ok(Ok(IndexConfig::postgres(url.clone())));
    }
    let path = config.data_dir.join(".vellum").join("index.sqlite");
    if let Some(hex) = var(KEY_VAR) {
        let key = index_key(Some(&hex)).map_err(|e| Error::new(ErrorKind::InvalidInput, e))?;
        return Ok(Ok(IndexConfig::sqlite(path, Some(key))));
    }
    if var(PLAINTEXT_VAR).is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes")) {
        return Ok(Ok(IndexConfig::sqlite(path, None)));
    }
    Ok(Err(format!(
        "no index is configured: set {KEY_VAR}, {PLAINTEXT_VAR}=1 or a database URL"
    )))
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

/// Logs Work events until the event log service takes them.
#[derive(Debug)]
struct TracingSink;

impl EventSink for TracingSink {
    fn publish(&self, events: Vec<WorkEvent>) {
        for event in events {
            tracing::debug!(?event, "work event");
        }
    }
}

/// The opened services, plus the two the file watcher drives.
struct Workspace {
    services: Vec<Arc<dyn Service>>,
    drive: Arc<Drive>,
    books: Arc<BooksService>,
    tasks: Arc<TasksService>,
}

/// The opened workspace services, and My Work for its plain HTTP routes.
pub struct Opened {
    /// Every typed service.
    pub services: Vec<Arc<dyn Service>>,
    /// My Work, which also serves the feed, CalDAV and capture routes.
    pub tasks: Option<Arc<TasksService>>,
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

/// Builds the services over `store`, `authz` and the drive's `index`.
async fn build<S: AuthzStore + TreeSync + 'static>(
    store: Arc<dyn BlobStore>,
    authz: Arc<Authz<S>>,
    index: Arc<dyn DriveIndex>,
    spaces: &Arc<Spaces>,
    identity: &Arc<Identity>,
    setup: TasksSetup,
) -> Result<Workspace, Error> {
    let drive = Arc::new(Drive::new(
        Arc::clone(&store),
        Arc::clone(&index),
        Arc::new(vellum_drive::authz::AuthzAccess::new(
            Arc::clone(&authz),
            Arc::clone(&index),
        )),
    ));
    let lookup = Arc::clone(spaces);
    let space_of: vellum_collections::service::SpaceOf =
        Arc::new(move |path: &BlobPath| lookup.of(path));
    let collections = CollectionsService::new(
        Arc::clone(&store),
        Arc::new(vellum_collections::service::AuthzAccess::new(
            Arc::clone(&authz),
            Arc::clone(&space_of),
        )),
    );
    let books = Arc::new(BooksService::new(
        Arc::clone(&store),
        Arc::new(vellum_books_service::AuthzAccess::new(
            Arc::clone(&authz),
            space_of,
        )),
    ));
    let (work, report) = WorkService::open(
        Arc::clone(&store),
        Arc::new(IdentityDirectory(Arc::clone(identity))),
        Arc::new(TracingSink),
        Arc::new(SystemClock),
    )
    .await?;
    if report.skipped > 0 {
        tracing::warn!(
            skipped = report.skipped,
            "some Work files could not be read"
        );
    }
    let work = Arc::new(work);
    let adapter = DriveTasks::open(
        Parts {
            drive: Arc::clone(&drive),
            store,
            index,
            authz,
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
    Ok(Workspace {
        services: vec![
            Arc::new(DriveService::new(Arc::clone(&drive))),
            Arc::new(collections),
            Arc::clone(&books) as Arc<dyn Service>,
            work,
            Arc::clone(&tasks) as Arc<dyn Service>,
        ],
        drive,
        books,
        tasks,
    })
}

/// Opens the workspace services for `config`, or returns none (with a
/// warning) when no index is configured.
///
/// # Errors
///
/// An unreachable database, a wrong key or an unreadable data folder.
pub async fn open(
    config: &Config,
    identity: &Arc<Identity>,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Opened, Error> {
    let setup = TasksSetup::new(identity, Some(&config.data_dir), &var);
    let index_config = match index_config(config, var)? {
        Ok(c) => c,
        Err(why) => {
            tracing::warn!("{why}; serving sign-in only, without the workspace API");
            return Ok(Opened {
                services: Vec::new(),
                tasks: None,
            });
        }
    };
    let index = Index::open(&index_config)
        .await
        .map_err(|e| unavailable("cannot open the index", e))?;
    let store: Arc<dyn BlobStore> = Arc::new(
        LocalStore::open(&config.data_dir)
            .await
            .map_err(|e| unavailable("cannot open the data folder", e))?,
    );
    let authz = Arc::new(Authz::new(SqlStore::new(index.clone())));
    let drive_index = Arc::new(SqlIndex::new(index));
    let spaces = Arc::new(Spaces::default());
    let workspace = build(
        Arc::clone(&store),
        authz,
        Arc::clone(&drive_index) as Arc<dyn DriveIndex>,
        &spaces,
        identity,
        setup,
    )
    .await?;
    // The watcher starts before the rescan, so nothing edited in between
    // is missed; whatever it sees twice is applied idempotently.
    crate::watcher::start(&config.data_dir, &store, &workspace.drive, &workspace.books).await?;
    match workspace.drive.scan().await {
        Ok(report) => tracing::info!(?report, "drive index rebuilt from the data folder"),
        Err(error) => tracing::warn!(%error, "drive rescan failed; the index may be stale"),
    }
    spaces.replace(load_spaces(drive_index.as_ref()).await?);
    let refresh = Arc::clone(&spaces);
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
    let timer = Arc::clone(&workspace.tasks);
    drop(tokio::spawn(async move {
        let mut tick = tokio::time::interval(GOOGLE_SYNC_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let _ = tick.tick().await;
            if let Err(error) = timer.google_sync_all().await {
                tracing::warn!(%error, "Google Calendar sync skipped");
            }
        }
    }));
    Ok(Opened {
        services: workspace.services,
        tasks: Some(workspace.tasks),
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
    let workspace = build(
        store,
        authz,
        index,
        &Arc::new(Spaces::default()),
        identity,
        setup,
    )
    .await?;
    Ok(workspace.services)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn path(p: &str) -> BlobPath {
        BlobPath::new(p).unwrap()
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
    fn index_needs_a_key_plaintext_or_a_database() {
        let config = Config::default();
        let vars = |pairs: &[(&str, &str)]| -> HashMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect()
        };
        let none = vars(&[]);
        assert!(
            index_config(&config, |k| none.get(k).cloned())
                .unwrap()
                .is_err()
        );
        let plain = vars(&[(PLAINTEXT_VAR, "1")]);
        assert!(matches!(
            index_config(&config, |k| plain.get(k).cloned()).unwrap(),
            Ok(IndexConfig::Sqlite { key: None, .. })
        ));
        let keyed = vars(&[(KEY_VAR, &"ab".repeat(32))]);
        assert!(matches!(
            index_config(&config, |k| keyed.get(k).cloned()).unwrap(),
            Ok(IndexConfig::Sqlite { key: Some(_), .. })
        ));
        let bad = vars(&[(KEY_VAR, "nope")]);
        assert!(index_config(&config, |k| bad.get(k).cloned()).is_err());
    }
}
