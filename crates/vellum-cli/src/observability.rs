// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The admin console behind `vellum serve`: the request recorder, route
//! metrics, usage counts, the audit log, admin settings and backups.
//!
//! One [`Observability`] is shared by three parts of the server. The
//! recorder layer (installed with telemetry) fills the trace buffer, the
//! gateway records rate, errors and duration per route, and the admin
//! service reads both for Admin → Requests and Admin → Routes.
//!
//! The admin service keeps its state in the data folder:
//!
//! - `.vellum/audit/`: the sealed, hash-chained audit log;
//! - `.vellum/admin/`: settings and the last backup and drill;
//! - `.vellum/backups/`: backups, unless the settings name another folder;
//! - `.vellum/keys/admin.key`: the key that seals the audit log, unless
//!   `VELLUM_ADMIN_KEY` is set;
//! - `.vellum/keys/backup.key`: the backup key, unless `VELLUM_BACKUP_KEY`
//!   is set. Backups leave the server, so their key should live elsewhere
//!   too; the server warns while it is kept next to the data.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use vellum_admin::api::{AdminApi, USAGE_MAX_AGE_MS, UsageFactsSource};
use vellum_admin::counts::{CountsSource, UsageCounts, UsageSampler};
use vellum_admin::usage::{HttpUsageTransport, UsageFacts, UsageTransport};
use vellum_admin::{
    AdminPaths, AdminService, FileAuditLog, FileSettingsStore, MemoryAuditLog,
    MemorySettingsStore,
};
use vellum_contracts::Service;
use vellum_crypto::{KekId, LocalKeyring, SecretKey};
use vellum_drive::{DriveIndex, DriveTotals};
use vellum_identity::Identity;
use vellum_kernel::config::Profile;
use vellum_kernel::{Config, Error, ErrorKind, Timestamp};
use vellum_observe::{
    RequestRecorder, RouteMetrics, TraceStore, recorder_layer,
};

use crate::serve::key_file;
use crate::services::VERSION;

/// Overrides the key that seals the audit log (64 hex characters).
pub const ADMIN_KEY_VAR: &str = "VELLUM_ADMIN_KEY";
/// Overrides the backup key (64 hex characters).
pub const BACKUP_KEY_VAR: &str = "VELLUM_BACKUP_KEY";

/// How often scheduled backups, restore drills and usage reports are
/// checked.
const TICK: Duration = Duration::from_mins(5);

/// How often the usage counts are taken. Exports and Admin → Usage read
/// the last sample in between.
const SAMPLE_EVERY: Duration = Duration::from_mins(5);

/// The trace buffer and route metrics shared by the recorder, the gateway
/// and the admin console.
#[derive(Debug, Clone, Default)]
pub struct Observability {
    /// Recent request traces (Admin → Requests).
    pub traces: Arc<TraceStore>,
    /// Rate, errors, duration and in-flight requests per route.
    pub metrics: RouteMetrics,
}

impl Observability {
    /// A fresh, empty buffer and metrics window.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The `tracing` layer that records every request into the buffer.
    #[must_use]
    pub fn recorder(&self) -> RequestRecorder {
        recorder_layer(Arc::clone(&self.traces))
    }
}

/// Counts the anonymous usage report may include. The deployment shape is
/// known from the configuration; users, spaces and documents come from the
/// usage sampler's latest counts, when the server keeps one.
#[derive(Debug, Clone)]
struct ServerFacts {
    profile: &'static str,
    database: &'static str,
    sampler: Option<Arc<UsageSampler>>,
}

impl ServerFacts {
    const fn of(config: &Config) -> Self {
        Self {
            profile: match config.profile {
                Profile::Team => "compose",
                Profile::Org => "helm",
                _ => "single",
            },
            database: if config.database_url.is_some() {
                "postgres"
            } else {
                "sqlite"
            },
            sampler: None,
        }
    }
}

#[async_trait]
impl UsageFactsSource for ServerFacts {
    async fn facts(&self) -> UsageFacts {
        let counts = match &self.sampler {
            Some(sampler) => sampler
                .current(USAGE_MAX_AGE_MS)
                .await
                .unwrap_or_else(|_| sampler.latest()),
            None => UsageCounts::default(),
        };
        UsageFacts {
            profile: self.profile.to_owned(),
            database: self.database.to_owned(),
            users: counts.active_users_30d,
            spaces: counts.spaces,
            documents: counts.documents(),
            ..UsageFacts::default()
        }
    }

    fn sampler(&self) -> Option<Arc<UsageSampler>> {
        self.sampler.clone()
    }
}

/// Counts people from identity and pages, storage and activity from the
/// drive index (D194). Without a drive index only people are counted.
pub struct WorkspaceCounts {
    /// Accounts, sessions and tokens.
    pub identity: Arc<Identity>,
    /// The drive index, when the workspace is open.
    pub drive: Option<Arc<dyn DriveIndex>>,
}

#[async_trait]
impl CountsSource for WorkspaceCounts {
    async fn count(&self, now: Timestamp) -> Result<UsageCounts, Error> {
        let people = self.identity.people(now);
        let drive = match &self.drive {
            Some(drive) => drive.totals(now).await.map_err(|e| {
                Error::new(ErrorKind::Unavailable, "cannot count the drive")
                    .with_source(e)
            })?,
            None => DriveTotals::default(),
        };
        Ok(UsageCounts {
            sampled_at_ms: now.unix_ms(),
            users: people.users,
            enabled_users: people.enabled,
            admins: people.admins,
            active_users_1d: people.active_1d,
            active_users_7d: people.active_7d,
            active_users_30d: people.active_30d,
            sessions: people.sessions,
            api_tokens: people.api_tokens,
            spaces: drive.spaces,
            folders: drive.folders,
            pages: drive.pages,
            books: drive.books,
            collections: drive.collections,
            trashed: drive.trashed,
            versions: drive.versions,
            file_bytes: drive.file_bytes,
            version_bytes: drive.version_bytes,
            events_1d: drive.events_1d,
            events_7d: drive.events_7d,
        })
    }
}

/// The usage sampler over `counts`, with its gauges registered, and the
/// facts source that reads it.
pub fn counted(
    config: &Config,
    counts: Arc<dyn CountsSource>,
) -> (Arc<UsageSampler>, Arc<dyn UsageFactsSource>) {
    let sampler = Arc::new(UsageSampler::new(counts));
    sampler.register_metrics();
    let facts = ServerFacts {
        sampler: Some(Arc::clone(&sampler)),
        ..ServerFacts::of(config)
    };
    (sampler, Arc::new(facts))
}

/// Takes the usage counts now and then every few minutes, for as long as
/// the server runs.
pub fn spawn_sampling(sampler: Arc<UsageSampler>) {
    drop(tokio::spawn(async move {
        let mut tick = tokio::time::interval(SAMPLE_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let _ = tick.tick().await;
            match sampler.refresh().await {
                Ok(counts) => tracing::debug!(?counts, "usage counted"),
                Err(error) => {
                    tracing::warn!(%error, "cannot count usage; keeping the \
                        last counts")
                }
            }
        }
    }));
}

/// A keyring holding one key, named `id`.
///
/// # Errors
///
/// The key is not 32 bytes.
pub fn keyring(id: &str, bytes: &[u8]) -> Result<LocalKeyring, Error> {
    let crypto = |e: vellum_crypto::CryptoError| {
        Error::new(ErrorKind::Internal, format!("cannot load the {id} key"))
            .with_source(e)
    };
    let kek = SecretKey::from_slice(bytes).map_err(crypto)?;
    Ok(LocalKeyring::new(KekId::new(id).map_err(crypto)?, kek))
}

/// The key named by `var`, or the one kept in the data folder under
/// `file`. Returns whether it came from the data folder.
fn load_key(
    data_dir: &Path,
    var: &str,
    file: &str,
    env: &impl Fn(&str) -> Option<String>,
) -> Result<(Vec<u8>, bool), Error> {
    env(var).map_or_else(
        || key_file(data_dir, file).map(|k| (k, true)),
        |hex| {
            crate::reindex::parse_hex_key(var, &hex)
                .map(|k| (k.to_vec(), false))
                .map_err(|why| Error::new(ErrorKind::InvalidInput, why))
        },
    )
}

/// Opens the admin service over the data folder.
///
/// # Errors
///
/// A malformed key, an unreadable folder, or an audit log whose chain is
/// broken (the server refuses to start rather than append after tampering).
pub async fn open(
    config: &Config,
    observability: &Observability,
    env: impl Fn(&str) -> Option<String>,
) -> Result<(Arc<AdminService<LocalKeyring>>, Arc<dyn UsageFactsSource>), Error>
{
    let data_dir = &config.data_dir;
    let (admin_key, _) = load_key(data_dir, ADMIN_KEY_VAR, "admin.key", &env)?;
    let (backup_key, beside_data) =
        load_key(data_dir, BACKUP_KEY_VAR, "backup.key", &env)?;
    if beside_data {
        tracing::warn!(
            "the backup key is kept in the data folder \
             (.vellum/keys/backup.key); copy it somewhere safe and set \
             {BACKUP_KEY_VAR}, or a lost server takes its backups with it"
        );
    }
    let vellum = data_dir.join(".vellum");
    let audit = FileAuditLog::open(
        vellum.join("audit"),
        Arc::new(keyring("admin", &admin_key)?),
    )
    .await?;
    let usage: Option<Arc<dyn UsageTransport>> = match HttpUsageTransport::new()
    {
        Ok(transport) => Some(Arc::new(transport)),
        Err(error) => {
            tracing::warn!(%error, "usage reports are off: no HTTP client");
            None
        }
    };
    let service = AdminService::new(
        Arc::new(audit),
        Arc::new(FileSettingsStore::new(
            vellum.join("admin").join("settings.json"),
        )),
        Arc::new(keyring("backup", &backup_key)?),
        Arc::clone(&observability.traces),
        observability.metrics.clone(),
        AdminPaths {
            data_dir: data_dir.clone(),
            scratch_dir: vellum.join("scratch"),
        },
        usage,
        VERSION,
    );
    Ok((Arc::new(service), Arc::new(ServerFacts::of(config))))
}

/// The admin API over the opened service.
#[must_use]
pub fn api(
    service: Arc<AdminService<LocalKeyring>>,
    facts: Arc<dyn UsageFactsSource>,
) -> Arc<dyn Service> {
    Arc::new(AdminApi::new(service, facts))
}

/// The admin API over in-memory stores and a throwaway key, for printing
/// the API contracts.
///
/// # Errors
///
/// Never in practice; the key and its id are constants.
pub fn describe(scratch: &Path) -> Result<Arc<dyn Service>, Error> {
    let observability = Observability::new();
    let service = AdminService::new(
        Arc::new(MemoryAuditLog::default()),
        Arc::new(MemorySettingsStore::default()),
        Arc::new(keyring("backup", &[0; 32])?),
        Arc::clone(&observability.traces),
        observability.metrics,
        AdminPaths {
            data_dir: scratch.to_path_buf(),
            scratch_dir: scratch.join("scratch"),
        },
        None,
        VERSION,
    );
    Ok(api(Arc::new(service), Arc::new(vellum_admin::api::NoFacts)))
}

/// Runs scheduled backups, restore drills and usage reports every few
/// minutes, for as long as the server runs.
pub fn spawn_ticks(
    service: Arc<AdminService<LocalKeyring>>,
    facts: Arc<dyn UsageFactsSource>,
) {
    drop(tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let _ = tick.tick().await;
            let facts = facts.facts().await;
            match service.tick(&facts).await {
                Ok(report) => tracing::debug!(?report, "admin jobs checked"),
                Err(error) => {
                    tracing::warn!(%error, "a scheduled backup, drill or \
                        report failed")
                }
            }
        }
    }));
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn config(dir: &Path) -> Config {
        let mut config = Config::default();
        config.data_dir = dir.to_path_buf();
        config
    }

    #[tokio::test]
    async fn keys_come_from_the_environment_or_the_data_folder()
    -> Result<(), Error> {
        let dir = tempfile::tempdir()
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        let vars: HashMap<&str, String> =
            HashMap::from([(BACKUP_KEY_VAR, "ab".repeat(32))]);
        let env = |k: &str| vars.get(k).cloned();
        let _ = open(&config(dir.path()), &Observability::new(), env).await?;
        let keys = dir.path().join(".vellum").join("keys");
        // The audit key is generated; the backup key came from the variable.
        assert!(keys.join("admin.key").is_file());
        assert!(!keys.join("backup.key").exists());
        // Reopening reads the same audit key, so the chain still opens.
        let _ = open(&config(dir.path()), &Observability::new(), env).await?;
        Ok(())
    }

    #[tokio::test]
    async fn a_malformed_key_stops_the_server() {
        let dir = tempfile::tempdir().ok();
        let dir = dir
            .as_ref()
            .map_or_else(|| Path::new("."), tempfile::TempDir::path);
        let env = |k: &str| (k == ADMIN_KEY_VAR).then(|| "not hex".to_owned());
        let error = open(&config(dir), &Observability::new(), env)
            .await
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(error.contains(ADMIN_KEY_VAR), "{error}");
    }

    #[tokio::test]
    async fn the_report_carries_the_real_counts() -> Result<(), Error> {
        let url = url::Url::parse("https://vellum.northstar.dev")
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        let identity = Arc::new(Identity::new(
            vellum_identity::IdentityConfig::new(url),
            vellum_identity::Store::memory(),
        ));
        let _ = identity.setup(
            "priya@northstar.dev",
            "Priya",
            "correct horse battery staple",
        )?;
        let _ = identity.sign_in(
            "priya@northstar.dev",
            "correct horse battery staple",
            None,
            None,
        )?;
        let counts = Arc::new(WorkspaceCounts {
            identity,
            drive: Some(Arc::new(vellum_drive::memory::MemoryIndex::new())),
        });
        let (sampler, facts) = counted(&config(Path::new(".")), counts);
        let report = facts.facts().await;
        assert_eq!((report.users, report.spaces, report.documents), (1, 0, 0));
        let latest = sampler.latest();
        assert!(latest.sampled_at_ms > 0);
        assert_eq!((latest.users, latest.admins, latest.sessions), (1, 1, 1));
        assert!(facts.sampler().is_some());
        Ok(())
    }

    #[test]
    fn the_report_names_the_deployment_shape() {
        let mut config = config(Path::new("."));
        config.database_url = Some("postgres://localhost/vellum".to_owned());
        let facts = ServerFacts::of(&config);
        assert_eq!((facts.profile, facts.database), ("single", "postgres"));
    }
}
