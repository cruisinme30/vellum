// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The workspace services: the drive, collections, books and Work, over the
//! files in the data folder, the index database and the permission engine.
//!
//! `vellum serve` opens them when an index is configured: Postgres through
//! the kernel's `database_url`, or the SQLite index in the data folder,
//! encrypted with `VELLUM_INDEX_KEY` (or plaintext when
//! `VELLUM_INDEX_PLAINTEXT=1`). Without one the server still starts, with
//! sign-in only, and says why.
//!
//! Workspace files live in the data folder, or in a shared S3 bucket when
//! `VELLUM_BLOB_URL` names one (`s3://bucket[/prefix]`, with
//! `VELLUM_S3_ENDPOINT`, `VELLUM_S3_REGION` and the `AWS_*` keys). Several
//! replicas need the bucket and a Postgres index, so they all see one
//! workspace and a restarted pod loses nothing.
//!
//! At startup the drive rescans the files. For a data folder, a file
//! watcher then keeps the index in step with edits made outside Vellum
//! (see [`crate::watcher`]); a bucket is only written through Vellum.

use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures::StreamExt as _;
use vellum_authz::{Authz, AuthzStore, Id, MemoryStore, SqlStore};
use vellum_books_service::BooksService;
use vellum_collections::CollectionsService;
use vellum_contracts::{BoxFuture, Operation, Readiness, Service};
use vellum_db::{Index, IndexConfig};
use vellum_drive::authz::TreeSync;
use vellum_drive::sql::SqlIndex;
use vellum_drive::{Drive, DriveIndex, DriveService};
use vellum_identity::Identity;
use vellum_kernel::{Config, Error, ErrorKind, UserId};
use vellum_store::remote::s3::{S3Config, S3Store};
use vellum_store::{BlobPath, BlobStore, LocalStore, StoreError};
use vellum_work::{UserRef, WorkEvent};
use vellum_work_service::{Directory, EventSink, SystemClock, WorkService};

use crate::reindex::{KEY_VAR, index_key};

/// Variable that opens the SQLite index without encryption.
pub const PLAINTEXT_VAR: &str = "VELLUM_INDEX_PLAINTEXT";

/// Variable naming the shared blob store; unset keeps files in the data
/// folder.
pub const BLOB_URL_VAR: &str = "VELLUM_BLOB_URL";

/// Folder the readiness probe lists. It is normally empty; any answer but
/// an error means the store is reachable. Listing, unlike a lookup, does
/// not log a missing file as an error on every probe.
const PROBE: &str = ".vellum/ready";

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
fn bucket(var: impl Fn(&str) -> Option<String>) -> Result<Option<S3Config>, Error> {
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
            store: Arc::new(store),
            local: true,
        });
    };
    let store = S3Store::new(bucket).map_err(|e| unavailable("cannot use the blob store", e))?;
    Ok(Files {
        store: Arc::new(store),
        local: false,
    })
}

/// Reports whether the blob store answers, for `/readyz`. It has no
/// operations of its own.
#[derive(Debug)]
struct Storage(Arc<dyn BlobStore>);

impl Service for Storage {
    fn name(&self) -> &'static str {
        "storage"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        Vec::new()
    }

    fn readiness(&self) -> BoxFuture<'_, Readiness> {
        Box::pin(probe(self.0.as_ref()))
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
}

/// Builds the services over `store`, `authz` and the drive's `index`.
async fn build<S: AuthzStore + TreeSync + 'static>(
    store: Arc<dyn BlobStore>,
    authz: Arc<Authz<S>>,
    index: Arc<dyn DriveIndex>,
    spaces: &Arc<Spaces>,
    identity: &Arc<Identity>,
) -> Result<Workspace, Error> {
    let drive = Arc::new(Drive::new(
        Arc::clone(&store),
        Arc::clone(&index),
        Arc::new(vellum_drive::authz::AuthzAccess::new(
            Arc::clone(&authz),
            index,
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
        Arc::new(vellum_books_service::AuthzAccess::new(authz, space_of)),
    ));
    let (work, report) = WorkService::open(
        store,
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
    Ok(Workspace {
        services: vec![
            Arc::new(DriveService::new(Arc::clone(&drive))),
            Arc::new(collections),
            Arc::clone(&books) as Arc<dyn Service>,
            Arc::new(work),
        ],
        drive,
        books,
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
) -> Result<Vec<Arc<dyn Service>>, Error> {
    let bucket = bucket(&var)?;
    let index_config = match index_config(config, var)? {
        Ok(c) => c,
        Err(why) => {
            tracing::warn!("{why}; serving sign-in only, without the workspace API");
            return Ok(Vec::new());
        }
    };
    let index = Index::open(&index_config)
        .await
        .map_err(|e| unavailable("cannot open the index", e))?;
    let Files { store, local } = open_files(config, bucket).await?;
    if !local && matches!(index_config, IndexConfig::Sqlite { .. }) {
        tracing::warn!(
            "files are in a shared bucket but the index is this machine's SQLite file; \
             run one replica only, or set a database URL"
        );
    }
    tracing::info!(backend = store.backend(), "workspace files opened");
    let authz = Arc::new(Authz::new(SqlStore::new(index.clone())));
    let drive_index = Arc::new(SqlIndex::new(index));
    let spaces = Arc::new(Spaces::default());
    let workspace = build(
        Arc::clone(&store),
        authz,
        Arc::clone(&drive_index) as Arc<dyn DriveIndex>,
        &spaces,
        identity,
    )
    .await?;
    // The watcher starts before the rescan, so nothing edited in between
    // is missed; whatever it sees twice is applied idempotently.
    if local {
        crate::watcher::start(&config.data_dir, &store, &workspace.drive, &workspace.books).await?;
    }
    match workspace.drive.scan().await {
        Ok(report) => tracing::info!(?report, "drive index rebuilt from the workspace files"),
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
    let mut services = workspace.services;
    services.push(Arc::new(Storage(store)));
    Ok(services)
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
    let workspace = build(store, authz, index, &Arc::new(Spaces::default()), identity).await?;
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
            &[(BLOB_URL_VAR, "s3://vellum")][..],
        ] {
            let error = bucket(lookup(vars)).err().unwrap();
            assert_eq!(error.kind(), ErrorKind::InvalidInput);
            assert!(!error.to_string().contains("secret"));
        }
    }

    #[tokio::test]
    async fn an_unreachable_bucket_is_not_ready() {
        // Port 9 (discard) refuses connections on a test machine.
        let store = S3Config::from_vars(lookup(&[
            (BLOB_URL_VAR, "s3://vellum"),
            ("VELLUM_S3_ENDPOINT", "http://127.0.0.1:9"),
            ("AWS_ACCESS_KEY_ID", "id"),
            ("AWS_SECRET_ACCESS_KEY", "secret"),
        ]))
        .and_then(S3Store::new)
        .unwrap();
        assert!(
            matches!(probe(&store).await, Readiness::NotReady(reason) if reason.contains("s3"))
        );
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
