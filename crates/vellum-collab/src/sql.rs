// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The update log in the index database (`collab_updates`, SQLite or
//! Postgres), sealed at rest.
//!
//! The database assigns `seq` (one past the room's highest), so two nodes
//! that both think they own a room cannot write the same position: the
//! loser's insert fails on the primary key and its writer retries. Seeding
//! relies on the same key, so a log is imported from Markdown only once.
//!
//! Every payload is the room's two IDs followed by the Yjs update, passed
//! through a [`Sealer`]. [`Envelope`] encrypts it with `vellum-crypto`
//! (a fresh data key per record, wrapped by the key provider); the ID
//! prefix binds a record to its document, so a record copied into another
//! document's log is refused. [`Plaintext`] is for spaces stored in clear.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use vellum_crypto::KeyProvider;
use vellum_db::sqlx::{self, Row as _};
use vellum_db::{BeginWrite as _, Index, IndexPool, traced};
use vellum_kernel::{Error, ErrorKind, Result};

use crate::hub::RoomKey;
use crate::ports::{Seq, StoredUpdate, UpdateStore};

/// Length of the document binding in front of each payload.
const BINDING_LEN: usize = 32;

/// Seals log records before they are written and opens them when read.
#[async_trait]
pub trait Sealer: Send + Sync + 'static {
    /// Turns a plaintext record into the bytes to store.
    async fn seal(&self, plaintext: Vec<u8>) -> Result<Vec<u8>>;

    /// Turns stored bytes back into the plaintext record.
    async fn open(&self, stored: Vec<u8>) -> Result<Vec<u8>>;
}

/// Envelope encryption with `vellum-crypto`.
#[derive(Debug)]
pub struct Envelope<K>(pub K);

#[async_trait]
impl<K: KeyProvider + 'static> Sealer for Envelope<K> {
    async fn seal(&self, plaintext: Vec<u8>) -> Result<Vec<u8>> {
        vellum_crypto::seal(&self.0, &plaintext)
            .await
            .map_err(|err| {
                Error::new(ErrorKind::Internal, "sealing a collab record failed").with_source(err)
            })
    }

    async fn open(&self, stored: Vec<u8>) -> Result<Vec<u8>> {
        vellum_crypto::open(&self.0, &stored).await.map_err(|err| {
            Error::new(ErrorKind::Internal, "opening a collab record failed").with_source(err)
        })
    }
}

/// No encryption, for spaces stored in clear. Refuses to read sealed
/// records rather than hand ciphertext to the CRDT.
#[derive(Debug, Clone, Copy, Default)]
pub struct Plaintext;

#[async_trait]
impl Sealer for Plaintext {
    async fn seal(&self, plaintext: Vec<u8>) -> Result<Vec<u8>> {
        Ok(plaintext)
    }

    async fn open(&self, stored: Vec<u8>) -> Result<Vec<u8>> {
        if vellum_crypto::is_envelope(&stored) {
            return Err(Error::new(
                ErrorKind::Internal,
                "a collab record is encrypted but no key provider is configured",
            ));
        }
        Ok(stored)
    }
}

/// [`UpdateStore`] over the index database.
#[derive(Clone)]
pub struct SqlUpdateStore {
    index: Index,
    sealer: Arc<dyn Sealer>,
}

impl std::fmt::Debug for SqlUpdateStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqlUpdateStore")
            .field("backend", &self.index.backend())
            .finish_non_exhaustive()
    }
}

impl SqlUpdateStore {
    /// A log in `index` whose records pass through `sealer`. The index
    /// must be migrated (`Index::open` does it).
    #[must_use]
    pub fn new(index: Index, sealer: impl Sealer) -> Self {
        Self {
            index,
            sealer: Arc::new(sealer),
        }
    }

    const fn system(&self) -> &'static str {
        self.index.backend().otel_system()
    }

    async fn seal(&self, room: RoomKey, update: Vec<u8>) -> Result<Vec<u8>> {
        let mut plaintext = Vec::with_capacity(BINDING_LEN + update.len());
        plaintext.extend_from_slice(&binding(room));
        plaintext.extend_from_slice(&update);
        self.sealer.seal(plaintext).await
    }

    async fn open(&self, room: RoomKey, stored: Vec<u8>) -> Result<Vec<u8>> {
        let mut plaintext = self.sealer.open(stored).await?;
        if plaintext.get(..BINDING_LEN) != Some(binding(room).as_slice()) {
            return Err(Error::new(
                ErrorKind::Internal,
                "a collab record belongs to another document",
            ));
        }
        Ok(plaintext.split_off(BINDING_LEN))
    }
}

const fn binding(room: RoomKey) -> [u8; BINDING_LEN] {
    let mut out = [0; BINDING_LEN];
    let (space, doc) = out.split_at_mut(BINDING_LEN / 2);
    space.copy_from_slice(&room.space.as_u128().to_be_bytes());
    doc.copy_from_slice(&room.doc.as_u128().to_be_bytes());
    out
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or_default()
}

fn db_error(err: sqlx::Error) -> Error {
    Error::new(
        ErrorKind::Unavailable,
        "the collab update log is unavailable",
    )
    .with_source(err)
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .is_some_and(sqlx::error::DatabaseError::is_unique_violation)
}

fn to_seq(value: i64) -> Result<Seq> {
    Seq::try_from(value).map_err(|_| {
        Error::new(
            ErrorKind::Internal,
            "the collab update log holds a negative seq",
        )
    })
}

fn from_seq(seq: Seq) -> Result<i64> {
    i64::try_from(seq).map_err(|_| Error::new(ErrorKind::InvalidInput, "seq out of range"))
}

/// The same statements for both backends; only placeholders and ID
/// encodings differ (16-byte blobs in SQLite, UUIDs in Postgres).
macro_rules! statements {
    ($p:literal) => {
        Statements {
            load: concat!(
                "SELECT seq, payload FROM collab_updates WHERE space_id = ",
                $p,
                "1 AND doc_id = ",
                $p,
                "2 ORDER BY seq"
            ),
            append: concat!(
                "INSERT INTO collab_updates (space_id, doc_id, seq, payload, created_at) SELECT ",
                $p,
                "1, ",
                $p,
                "2, COALESCE(MAX(seq), 0) + 1, ",
                $p,
                "3, ",
                $p,
                "4 FROM collab_updates WHERE space_id = ",
                $p,
                "1 AND doc_id = ",
                $p,
                "2 RETURNING seq"
            ),
            seed: concat!(
                "INSERT INTO collab_updates (space_id, doc_id, seq, payload, created_at) SELECT ",
                $p,
                "1, ",
                $p,
                "2, 1, ",
                $p,
                "3, ",
                $p,
                "4 WHERE NOT EXISTS (SELECT 1 FROM collab_updates WHERE space_id = ",
                $p,
                "1 AND doc_id = ",
                $p,
                "2)"
            ),
            prune: concat!(
                "DELETE FROM collab_updates WHERE space_id = ",
                $p,
                "1 AND doc_id = ",
                $p,
                "2 AND seq <= ",
                $p,
                "3"
            ),
            insert: concat!(
                "INSERT INTO collab_updates (space_id, doc_id, seq, payload, created_at) VALUES (",
                $p,
                "1, ",
                $p,
                "2, ",
                $p,
                "3, ",
                $p,
                "4, ",
                $p,
                "5)"
            ),
        }
    };
}

struct Statements {
    load: &'static str,
    append: &'static str,
    seed: &'static str,
    prune: &'static str,
    insert: &'static str,
}

/// Runs one operation against whichever backend the index uses. `$space`
/// and `$doc` are bound as the backend's ID type.
macro_rules! with_pool {
    ($self:expr, $room:expr, |$pool:ident, $space:ident, $doc:ident, $sql:ident| $body:block) => {
        match $self.index.pool() {
            IndexPool::Sqlite($pool) => {
                let $space = $room.space.as_u128().to_be_bytes().to_vec();
                let $doc = $room.doc.as_u128().to_be_bytes().to_vec();
                let $sql = statements!("?");
                $body
            }
            IndexPool::Postgres($pool) => {
                let $space = $room.space.as_uuid();
                let $doc = $room.doc.as_uuid();
                let $sql = statements!("$");
                $body
            }
            _ => Err(Error::new(ErrorKind::Internal, "unsupported index backend")),
        }
    };
}

#[async_trait]
impl UpdateStore for SqlUpdateStore {
    #[tracing::instrument(name = "collab.log.load", skip_all, fields(room = %room))]
    async fn load(&self, room: RoomKey) -> Result<Vec<StoredUpdate>> {
        let system = self.system();
        let rows: Vec<(i64, Vec<u8>)> = with_pool!(self, room, |pool, space, doc, sql| {
            let query = sqlx::query(sql.load).bind(space).bind(doc);
            traced(system, "collab.load", query.fetch_all(pool))
                .await
                .map_err(db_error)?
                .iter()
                .map(|row| Ok((row.try_get(0)?, row.try_get(1)?)))
                .collect::<std::result::Result<_, sqlx::Error>>()
                .map_err(db_error)
        })?;
        let mut out = Vec::with_capacity(rows.len());
        for (seq, payload) in rows {
            out.push(StoredUpdate {
                seq: to_seq(seq)?,
                update: self.open(room, payload).await?,
            });
        }
        Ok(out)
    }

    #[tracing::instrument(name = "collab.log.append", skip_all, fields(room = %room, bytes = update.len()))]
    async fn append(&self, room: RoomKey, update: Vec<u8>) -> Result<Seq> {
        let system = self.system();
        let payload = self.seal(room, update).await?;
        let seq: i64 = with_pool!(self, room, |pool, space, doc, sql| {
            let query = sqlx::query_scalar(sql.append)
                .bind(space)
                .bind(doc)
                .bind(payload)
                .bind(now_ms());
            traced(system, "collab.append", query.fetch_one(pool))
                .await
                .map_err(db_error)
        })?;
        to_seq(seq)
    }

    #[tracing::instrument(name = "collab.log.seed", skip_all, fields(room = %room))]
    async fn seed(&self, room: RoomKey, update: Vec<u8>) -> Result<bool> {
        let system = self.system();
        let payload = self.seal(room, update).await?;
        with_pool!(self, room, |pool, space, doc, sql| {
            let query = sqlx::query(sql.seed)
                .bind(space)
                .bind(doc)
                .bind(payload)
                .bind(now_ms());
            match traced(system, "collab.seed", query.execute(pool)).await {
                Ok(done) => Ok(done.rows_affected() == 1),
                // Another node seeded first.
                Err(err) if is_unique_violation(&err) => Ok(false),
                Err(err) => Err(db_error(err)),
            }
        })
    }

    #[allow(
        clippy::clone_on_copy,
        reason = "the same statements bind owned blobs (SQLite) and Copy UUIDs (Postgres)"
    )]
    #[tracing::instrument(name = "collab.log.compact", skip_all, fields(room = %room, through))]
    async fn compact(&self, room: RoomKey, through: Seq, merged: Vec<u8>) -> Result<()> {
        let system = self.system();
        let through = from_seq(through)?;
        let payload = self.seal(room, merged).await?;
        with_pool!(self, room, |pool, space, doc, sql| {
            let mut tx = pool.begin_write().await.map_err(db_error)?;
            let prune = sqlx::query(sql.prune)
                .bind(space.clone())
                .bind(doc.clone())
                .bind(through);
            let _pruned = traced(system, "collab.compact.prune", prune.execute(&mut *tx))
                .await
                .map_err(db_error)?;
            let insert = sqlx::query(sql.insert)
                .bind(space)
                .bind(doc)
                .bind(through)
                .bind(payload)
                .bind(now_ms());
            let _inserted = traced(system, "collab.compact.insert", insert.execute(&mut *tx))
                .await
                .map_err(db_error)?;
            tx.commit().await.map_err(db_error)
        })
    }
}
