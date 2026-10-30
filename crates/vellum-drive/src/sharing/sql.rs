// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! [`SqlShareLinks`]: link records in the `vellum-db` index (migration
//! 0011). The SQL reads the same on SQLite and Postgres.

use std::fmt;

use async_trait::async_trait;
use vellum_db::sqlx::types::Uuid;
use vellum_db::{Index, IndexPool, sqlx, traced};
use vellum_kernel::{
    ObjectId, ShareId, ShareTokenHash, SpaceId, Timestamp, UserId,
};

use super::{LinkRecord, ShareLinks};
use crate::error::{DriveError, DriveResult};

/// Link records stored in the index.
#[derive(Clone)]
pub struct SqlShareLinks {
    db: Index,
}

impl fmt::Debug for SqlShareLinks {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SqlShareLinks")
            .field("backend", &self.db.backend())
            .finish_non_exhaustive()
    }
}

impl SqlShareLinks {
    /// Wraps an opened (and therefore migrated) index.
    #[must_use]
    pub const fn new(db: Index) -> Self {
        Self { db }
    }
}

type Row = (
    Uuid,
    Uuid,
    Uuid,
    String,
    Option<String>,
    Option<i64>,
    Uuid,
    i64,
    Option<i64>,
);

macro_rules! columns {
    () => {
        "link_id, space_id, node_id, token_hash, password_hash, expires_at, \
         created_by, created_at, revoked_at"
    };
}

fn db_error(err: impl fmt::Display) -> DriveError {
    DriveError::Index(err.to_string())
}

fn time(ms: i64) -> DriveResult<Timestamp> {
    Timestamp::from_unix_ms(ms).map_err(db_error)
}

fn record(row: Row) -> DriveResult<LinkRecord> {
    let (link, space, node, hash, password, expires, by, at, revoked) = row;
    Ok(LinkRecord {
        id: ShareId::from_u128(link.as_u128()),
        space: SpaceId::from_u128(space.as_u128()),
        node: ObjectId::from_u128(node.as_u128()),
        token_hash: hash.parse::<ShareTokenHash>().map_err(db_error)?,
        password_hash: password,
        expires_at: expires.map(time).transpose()?,
        created_by: UserId::from_u128(by.as_u128()),
        created_at: time(at)?,
        revoked_at: revoked.map(time).transpose()?,
    })
}

macro_rules! on_pool {
    ($self:ident, $pool:ident, $system:ident => $body:block) => {
        match $self.db.pool() {
            IndexPool::Sqlite($pool) => {
                let $system = "sqlite";
                $body
            }
            IndexPool::Postgres($pool) => {
                let $system = "postgresql";
                $body
            }
            _ => Err(DriveError::Index("unsupported index backend".to_owned())),
        }
    };
}

macro_rules! select {
    ($self:ident, $op:literal, $filter:literal, $bind:expr) => {{
        let rows: Vec<Row> = on_pool!($self, pool, system => {
            traced(
                system,
                concat!("drive.share_links.", $op),
                sqlx::query_as(concat!("SELECT ", columns!(), " FROM \
                    share_links WHERE ", $filter))
                    .bind($bind)
                    .fetch_all(pool),
            )
            .await
            .map_err(db_error)
        })?;
        rows.into_iter().map(record).collect::<DriveResult<Vec<_>>>()
    }};
}

#[async_trait]
impl ShareLinks for SqlShareLinks {
    async fn insert(&self, r: &LinkRecord) -> DriveResult<()> {
        on_pool!(self, pool, system => {
            traced(
                system,
                "drive.share_links.insert",
                sqlx::query(concat!(
                    "INSERT INTO share_links (",
                    columns!(),
                    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"
                ))
                .bind(Uuid::from_u128(r.id.as_u128()))
                .bind(Uuid::from_u128(r.space.as_u128()))
                .bind(Uuid::from_u128(r.node.as_u128()))
                .bind(r.token_hash.to_string())
                .bind(r.password_hash.as_deref())
                .bind(r.expires_at.map(Timestamp::unix_ms))
                .bind(Uuid::from_u128(r.created_by.as_u128()))
                .bind(r.created_at.unix_ms())
                .bind(r.revoked_at.map(Timestamp::unix_ms))
                .execute(pool),
            )
            .await
            .map(|_| ())
            .map_err(db_error)
        })
    }

    async fn of_node(&self, node: ObjectId) -> DriveResult<Vec<LinkRecord>> {
        let mut found = select!(
            self,
            "of_node",
            "node_id = $1 ORDER BY created_at DESC, link_id DESC",
            Uuid::from_u128(node.as_u128())
        )?;
        found.truncate(500);
        Ok(found)
    }

    async fn get(&self, id: ShareId) -> DriveResult<Option<LinkRecord>> {
        Ok(
            select!(
                self,
                "get",
                "link_id = $1",
                Uuid::from_u128(id.as_u128())
            )?
            .into_iter()
            .next(),
        )
    }

    async fn by_hash(
        &self,
        hash: &ShareTokenHash,
    ) -> DriveResult<Option<LinkRecord>> {
        Ok(
            select!(self, "by_hash", "token_hash = $1", hash.to_string())?
                .into_iter()
                .next(),
        )
    }

    async fn revoke(&self, id: ShareId, at: Timestamp) -> DriveResult<()> {
        on_pool!(self, pool, system => {
            traced(
                system,
                "drive.share_links.revoke",
                sqlx::query(
                    "UPDATE share_links SET revoked_at = $2 WHERE link_id = $1 \
                        AND revoked_at IS NULL",
                )
                .bind(Uuid::from_u128(id.as_u128()))
                .bind(at.unix_ms())
                .execute(pool),
            )
            .await
            .map(|_| ())
            .map_err(db_error)
        })
    }
}
