// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The [`SyncStore`] on the index database: SQLite for the single binary,
//! Postgres for the HA profiles.
//!
//! The tables (`sync_meta`, `sync_log`, `sync_clients`) come from the
//! `vellum-db` migrations. Every append first bumps `sync_meta.head` inside
//! its transaction; that row lock serialises writers, so positions commit
//! in order and a pull never skips one that commits late.

use serde_json::Value;
use vellum_db::sqlx::postgres::PgRow;
use vellum_db::sqlx::sqlite::SqliteRow;
use vellum_db::sqlx::types::Uuid;
use vellum_db::sqlx::{self, Row as _};
use vellum_db::{Index, IndexPool, traced};
use vellum_kernel::{Error, ErrorKind, ObjectId, SpaceId, Timestamp, UserId};

use crate::domain::{Change, ClientId, Record};
use crate::ports::{ClientState, LogInfo, SyncStore};

const SQLITE: &str = "sqlite";
const POSTGRES: &str = "postgresql";

/// The sync store on the index database.
#[derive(Debug, Clone)]
pub struct SqlStore {
    index: Index,
}

impl SqlStore {
    /// A store over a migrated index.
    #[must_use]
    pub const fn new(index: Index) -> Self {
        Self { index }
    }

    /// Drops tombstones at or below `seq` and raises the horizon to it, so
    /// clients with an older cursor reset instead of missing a delete.
    ///
    /// # Errors
    ///
    /// `unavailable` when the database cannot be reached.
    pub async fn prune_tombstones(&self, seq: u64) -> Result<(), Error> {
        let seq = to_i64(seq)?;
        match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let mut tx = pool.begin().await.map_err(db_error)?;
                let _ = traced(
                    SQLITE,
                    "sync.raise_horizon",
                    sqlx::query("UPDATE sync_meta SET horizon = max(horizon, ?) WHERE id = 1")
                        .bind(seq)
                        .execute(&mut *tx),
                )
                .await
                .map_err(db_error)?;
                let _ = traced(
                    SQLITE,
                    "sync.prune_tombstones",
                    sqlx::query("DELETE FROM sync_log WHERE seq <= ? AND value IS NULL")
                        .bind(seq)
                        .execute(&mut *tx),
                )
                .await
                .map_err(db_error)?;
                tx.commit().await.map_err(db_error)
            }
            IndexPool::Postgres(pool) => {
                let mut tx = pool.begin().await.map_err(db_error)?;
                let _ = traced(
                    POSTGRES,
                    "sync.raise_horizon",
                    sqlx::query(
                        "UPDATE sync_meta SET horizon = GREATEST(horizon, $1) WHERE id = 1",
                    )
                    .bind(seq)
                    .execute(&mut *tx),
                )
                .await
                .map_err(db_error)?;
                let _ = traced(
                    POSTGRES,
                    "sync.prune_tombstones",
                    sqlx::query("DELETE FROM sync_log WHERE seq <= $1 AND value IS NULL")
                        .bind(seq)
                        .execute(&mut *tx),
                )
                .await
                .map_err(db_error)?;
                tx.commit().await.map_err(db_error)
            }
            _ => Err(unsupported()),
        }
    }
}

impl SyncStore for SqlStore {
    async fn client(&self, client: &ClientId) -> Result<Option<ClientState>, Error> {
        let row = match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let row: Option<(Vec<u8>, i64)> = traced(
                    SQLITE,
                    "sync.client",
                    sqlx::query_as(
                        "SELECT user_id, last_mutation_id FROM sync_clients WHERE client_id = ?",
                    )
                    .bind(client.as_str())
                    .fetch_optional(pool),
                )
                .await
                .map_err(db_error)?;
                row.map(|(user, last)| Ok::<_, Error>((uuid_from_blob(&user)?, last)))
                    .transpose()?
            }
            IndexPool::Postgres(pool) => traced(
                POSTGRES,
                "sync.client",
                sqlx::query_as(
                    "SELECT user_id, last_mutation_id FROM sync_clients WHERE client_id = $1",
                )
                .bind(client.as_str())
                .fetch_optional(pool),
            )
            .await
            .map_err(db_error)?,
            _ => return Err(unsupported()),
        };
        row.map(|(user, last)| {
            Ok(ClientState {
                user: UserId::from_uuid(user),
                last_mutation_id: to_u64(last)?,
            })
        })
        .transpose()
    }

    async fn advance_client(
        &self,
        client: &ClientId,
        user: UserId,
        last_mutation_id: u64,
    ) -> Result<(), Error> {
        let last = to_i64(last_mutation_id)?;
        let now = Timestamp::now().unix_ms();
        let affected = match self.index.pool() {
            IndexPool::Sqlite(pool) => traced(
                SQLITE,
                "sync.advance_client",
                sqlx::query(
                    "INSERT INTO sync_clients (client_id, user_id, last_mutation_id, updated_at) \
                     VALUES (?, ?, ?, ?) ON CONFLICT (client_id) DO UPDATE SET \
                     last_mutation_id = max(sync_clients.last_mutation_id, excluded.last_mutation_id), \
                     updated_at = excluded.updated_at \
                     WHERE sync_clients.user_id = excluded.user_id",
                )
                .bind(client.as_str())
                .bind(user.as_uuid().as_bytes().as_slice())
                .bind(last)
                .bind(now)
                .execute(pool),
            )
            .await
            .map_err(db_error)?
            .rows_affected(),
            IndexPool::Postgres(pool) => traced(
                POSTGRES,
                "sync.advance_client",
                sqlx::query(
                    "INSERT INTO sync_clients (client_id, user_id, last_mutation_id, updated_at) \
                     VALUES ($1, $2, $3, $4) ON CONFLICT (client_id) DO UPDATE SET \
                     last_mutation_id = GREATEST(sync_clients.last_mutation_id, excluded.last_mutation_id), \
                     updated_at = excluded.updated_at \
                     WHERE sync_clients.user_id = excluded.user_id",
                )
                .bind(client.as_str())
                .bind(user.as_uuid())
                .bind(last)
                .bind(now)
                .execute(pool),
            )
            .await
            .map_err(db_error)?
            .rows_affected(),
            _ => return Err(unsupported()),
        };
        if affected == 0 {
            // The row exists and the WHERE clause refused it: another account owns it.
            Err(Error::new(
                ErrorKind::PermissionDenied,
                "this client belongs to another account",
            ))
        } else {
            Ok(())
        }
    }

    async fn append(&self, changes: Vec<Change>) -> Result<(), Error> {
        if changes.is_empty() {
            return Ok(());
        }
        let count = i64::try_from(changes.len())
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "too many changes"))?;
        let values = changes
            .iter()
            .map(|change| change.value.as_ref().map(Value::to_string))
            .collect::<Vec<_>>();
        match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let mut tx = pool.begin().await.map_err(db_error)?;
                let head: i64 = traced(
                    SQLITE,
                    "sync.reserve",
                    sqlx::query_scalar(
                        "UPDATE sync_meta SET head = head + ? WHERE id = 1 RETURNING head",
                    )
                    .bind(count)
                    .fetch_one(&mut *tx),
                )
                .await
                .map_err(db_error)?;
                for (seq, (change, value)) in (head - count + 1..).zip(changes.iter().zip(values)) {
                    let _ = traced(
                        SQLITE,
                        "sync.append",
                        sqlx::query(
                            "INSERT INTO sync_log \
                             (seq, space_id, object_id, collection, record_key, value) \
                             VALUES (?, ?, ?, ?, ?, ?) \
                             ON CONFLICT (space_id, collection, record_key) DO UPDATE SET \
                             seq = excluded.seq, object_id = excluded.object_id, \
                             value = excluded.value",
                        )
                        .bind(seq)
                        .bind(change.space.as_uuid().as_bytes().as_slice())
                        .bind(change.object.map(|id| id.as_uuid().as_bytes().to_vec()))
                        .bind(change.collection.as_str())
                        .bind(change.key.as_str())
                        .bind(value)
                        .execute(&mut *tx),
                    )
                    .await
                    .map_err(db_error)?;
                }
                tx.commit().await.map_err(db_error)
            }
            IndexPool::Postgres(pool) => {
                let mut tx = pool.begin().await.map_err(db_error)?;
                let head: i64 = traced(
                    POSTGRES,
                    "sync.reserve",
                    sqlx::query_scalar(
                        "UPDATE sync_meta SET head = head + $1 WHERE id = 1 RETURNING head",
                    )
                    .bind(count)
                    .fetch_one(&mut *tx),
                )
                .await
                .map_err(db_error)?;
                for (seq, (change, value)) in (head - count + 1..).zip(changes.iter().zip(values)) {
                    let _ = traced(
                        POSTGRES,
                        "sync.append",
                        sqlx::query(
                            "INSERT INTO sync_log \
                             (seq, space_id, object_id, collection, record_key, value) \
                             VALUES ($1, $2, $3, $4, $5, $6) \
                             ON CONFLICT (space_id, collection, record_key) DO UPDATE SET \
                             seq = excluded.seq, object_id = excluded.object_id, \
                             value = excluded.value",
                        )
                        .bind(seq)
                        .bind(change.space.as_uuid())
                        .bind(change.object.map(ObjectId::as_uuid))
                        .bind(change.collection.as_str())
                        .bind(change.key.as_str())
                        .bind(value)
                        .execute(&mut *tx),
                    )
                    .await
                    .map_err(db_error)?;
                }
                tx.commit().await.map_err(db_error)
            }
            _ => Err(unsupported()),
        }
    }

    async fn read(&self, after: u64, limit: usize, live_only: bool) -> Result<Vec<Record>, Error> {
        let after = to_i64(after)?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let rows = traced(
                    SQLITE,
                    "sync.read",
                    sqlx::query(
                        "SELECT seq, space_id, object_id, collection, record_key, value \
                         FROM sync_log WHERE seq > ? AND (? = 0 OR value IS NOT NULL) \
                         ORDER BY seq LIMIT ?",
                    )
                    .bind(after)
                    .bind(live_only)
                    .bind(limit)
                    .fetch_all(pool),
                )
                .await
                .map_err(db_error)?;
                rows.iter().map(sqlite_record).collect()
            }
            IndexPool::Postgres(pool) => {
                let rows = traced(
                    POSTGRES,
                    "sync.read",
                    sqlx::query(
                        "SELECT seq, space_id, object_id, collection, record_key, value \
                         FROM sync_log WHERE seq > $1 AND (NOT $2 OR value IS NOT NULL) \
                         ORDER BY seq LIMIT $3",
                    )
                    .bind(after)
                    .bind(live_only)
                    .bind(limit)
                    .fetch_all(pool),
                )
                .await
                .map_err(db_error)?;
                rows.iter().map(postgres_record).collect()
            }
            _ => Err(unsupported()),
        }
    }

    async fn info(&self) -> Result<LogInfo, Error> {
        const INFO: &str = "SELECT epoch, horizon, head FROM sync_meta WHERE id = 1";
        let (epoch, horizon, head): (i64, i64, i64) = match self.index.pool() {
            IndexPool::Sqlite(pool) => {
                let row = traced(SQLITE, "sync.info", sqlx::query(INFO).fetch_one(pool))
                    .await
                    .map_err(db_error)?;
                (
                    row.try_get("epoch").map_err(db_error)?,
                    row.try_get("horizon").map_err(db_error)?,
                    row.try_get("head").map_err(db_error)?,
                )
            }
            IndexPool::Postgres(pool) => {
                let row = traced(POSTGRES, "sync.info", sqlx::query(INFO).fetch_one(pool))
                    .await
                    .map_err(db_error)?;
                (
                    row.try_get("epoch").map_err(db_error)?,
                    row.try_get("horizon").map_err(db_error)?,
                    row.try_get("head").map_err(db_error)?,
                )
            }
            _ => return Err(unsupported()),
        };
        Ok(LogInfo {
            epoch: to_u64(epoch)?,
            horizon: to_u64(horizon)?,
            head: to_u64(head)?,
        })
    }
}

fn sqlite_record(row: &SqliteRow) -> Result<Record, Error> {
    let space: Vec<u8> = row.try_get("space_id").map_err(db_error)?;
    let object: Option<Vec<u8>> = row.try_get("object_id").map_err(db_error)?;
    record(
        row.try_get("seq").map_err(db_error)?,
        uuid_from_blob(&space)?,
        object.as_deref().map(uuid_from_blob).transpose()?,
        row.try_get("collection").map_err(db_error)?,
        row.try_get("record_key").map_err(db_error)?,
        row.try_get("value").map_err(db_error)?,
    )
}

fn postgres_record(row: &PgRow) -> Result<Record, Error> {
    record(
        row.try_get("seq").map_err(db_error)?,
        row.try_get("space_id").map_err(db_error)?,
        row.try_get("object_id").map_err(db_error)?,
        row.try_get("collection").map_err(db_error)?,
        row.try_get("record_key").map_err(db_error)?,
        row.try_get("value").map_err(db_error)?,
    )
}

fn record(
    seq: i64,
    space: Uuid,
    object: Option<Uuid>,
    collection: String,
    key: String,
    value: Option<String>,
) -> Result<Record, Error> {
    let value = value
        .map(|text| serde_json::from_str(&text))
        .transpose()
        .map_err(|err| corrupt("a sync record is not valid JSON").with_source(err))?;
    Ok(Record {
        change: Change {
            space: SpaceId::from_uuid(space),
            object: object.map(ObjectId::from_uuid),
            collection,
            key,
            value,
        },
        seq: to_u64(seq)?,
    })
}

fn uuid_from_blob(bytes: &[u8]) -> Result<Uuid, Error> {
    Uuid::from_slice(bytes).map_err(|err| corrupt("a sync ID is not 16 bytes").with_source(err))
}

fn to_i64(value: u64) -> Result<i64, Error> {
    i64::try_from(value).map_err(|_| Error::new(ErrorKind::InvalidInput, "number out of range"))
}

fn to_u64(value: i64) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| corrupt("a sync counter is negative"))
}

fn corrupt(message: &'static str) -> Error {
    Error::new(ErrorKind::Internal, message)
}

fn unsupported() -> Error {
    Error::new(ErrorKind::Internal, "the index backend is not supported")
}

/// Connection trouble is retryable; anything else is a bug or bad data.
fn db_error(err: sqlx::Error) -> Error {
    let kind = match &err {
        sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_) => {
            ErrorKind::Unavailable
        }
        sqlx::Error::Database(db) if db.code().as_deref() == Some("SQLITE_BUSY") => {
            ErrorKind::Unavailable
        }
        _ => ErrorKind::Internal,
    };
    Error::new(kind, "the sync store failed").with_source(err)
}
