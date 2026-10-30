// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Write transactions that never fail with "database is locked" (D306).
//!
//! On SQLite in WAL mode, a plain `BEGIN` is deferred: the transaction
//! starts as a reader and takes the write lock at its first write. If
//! another connection commits in between, the upgrade fails at once with
//! `SQLITE_BUSY_SNAPSHOT` ("database is locked"), and the busy timeout
//! cannot help because waiting never makes the old snapshot current.
//! `BEGIN IMMEDIATE` takes the write lock up front, so a writer waits in
//! the busy handler for its turn instead. Postgres keeps a plain `BEGIN`.

use std::future::Future;

use sqlx::{PgPool, Postgres, Sqlite, SqlitePool, Transaction};

/// Starts a transaction that will write. Use it, not `Pool::begin`, for
/// every transaction that writes to the index.
pub trait BeginWrite {
    /// The database the transaction runs on.
    type Database: sqlx::Database;

    /// Acquires a connection and begins a write transaction on it.
    fn begin_write(
        &self,
    ) -> impl Future<
        Output = Result<Transaction<'static, Self::Database>, sqlx::Error>,
    > + Send;
}

impl BeginWrite for SqlitePool {
    type Database = Sqlite;

    fn begin_write(
        &self,
    ) -> impl Future<Output = Result<Transaction<'static, Sqlite>, sqlx::Error>> + Send
    {
        self.begin_with("BEGIN IMMEDIATE")
    }
}

impl BeginWrite for PgPool {
    type Database = Postgres;

    fn begin_write(
        &self,
    ) -> impl Future<
        Output = Result<Transaction<'static, Postgres>, sqlx::Error>,
    > + Send {
        self.begin()
    }
}
