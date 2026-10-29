// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Errors raised by the index.

/// Result alias for index operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong talking to the index.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The database driver reported an error.
    #[error("index database error: {0}")]
    Database(#[from] sqlx::Error),
    /// Applying migrations failed.
    #[error("index migration failed: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// The SQLite file could not be opened with the given key (wrong key, or not an index).
    #[error("the index at {path} could not be unlocked with the configured key")]
    Locked {
        /// Path of the SQLite file.
        path: String,
    },
}
