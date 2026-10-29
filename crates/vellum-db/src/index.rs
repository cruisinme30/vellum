// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The index handle: one pool over the configured backend.

use std::str::FromStr as _;
use std::time::Duration;

use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};

use crate::config::IndexConfig;
use crate::error::{Error, Result};
use crate::telemetry::traced;

static SQLITE_MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations/sqlite");

/// Which database the index lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// SQLite, optionally SQLCipher-encrypted.
    Sqlite,
}

impl Backend {
    /// The OpenTelemetry `db.system` value.
    #[must_use]
    pub const fn otel_system(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
        }
    }
}

#[derive(Debug, Clone)]
enum Pool {
    Sqlite(SqlitePool),
}

/// A migrated, ready-to-use index. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Index {
    pool: Pool,
}

impl Index {
    /// Connects, unlocks (SQLCipher), and applies every pending migration.
    ///
    /// # Errors
    /// Fails if the database is unreachable, the key is wrong, or a migration fails.
    #[tracing::instrument(name = "db.open", skip_all, fields(db.system))]
    pub async fn open(config: &IndexConfig) -> Result<Self> {
        let index = match config {
            IndexConfig::Sqlite { path, key } => {
                let mut options = SqliteConnectOptions::from_str("sqlite://")?
                    .filename(path)
                    .create_if_missing(true)
                    .foreign_keys(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Normal)
                    .busy_timeout(Duration::from_secs(5));
                if let Some(key) = key {
                    options = options.pragma("key", key.pragma_value());
                }
                let pool = SqlitePoolOptions::new()
                    .max_connections(4)
                    .connect_with(options)
                    .await
                    .map_err(|err| match err {
                        // SQLITE_NOTADB: wrong key, or an encrypted file opened without one.
                        sqlx::Error::Database(db) if db.code().as_deref() == Some("26") => {
                            Error::Locked {
                                path: path.display().to_string(),
                            }
                        }
                        other => Error::Database(other),
                    })?;
                Self {
                    pool: Pool::Sqlite(pool),
                }
            }
        };
        let _ = tracing::Span::current().record("db.system", index.backend().otel_system());
        index.migrate().await?;
        Ok(index)
    }

    /// The backend this index runs on.
    #[must_use]
    pub const fn backend(&self) -> Backend {
        match self.pool {
            Pool::Sqlite(_) => Backend::Sqlite,
        }
    }

    /// Applies pending forward-only migrations. Safe to call repeatedly.
    ///
    /// # Errors
    /// Fails if a migration fails or a previously applied one was edited.
    pub async fn migrate(&self) -> Result<()> {
        let system = self.backend().otel_system();
        match &self.pool {
            Pool::Sqlite(pool) => traced(system, "migrate", SQLITE_MIGRATIONS.run(pool)).await?,
        }
        Ok(())
    }

    /// Round-trips a trivial query; used by `/readyz`.
    ///
    /// # Errors
    /// Fails if the database cannot be reached.
    pub async fn ping(&self) -> Result<()> {
        let system = self.backend().otel_system();
        match &self.pool {
            Pool::Sqlite(pool) => {
                let _ = traced(system, "ping", sqlx::query("SELECT 1").execute(pool)).await?;
            }
        }
        Ok(())
    }

    /// Closes every pooled connection.
    pub async fn close(&self) {
        match &self.pool {
            Pool::Sqlite(pool) => pool.close().await,
        }
    }
}
