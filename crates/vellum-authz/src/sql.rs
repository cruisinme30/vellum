// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The [`AuthzStore`] adapter over the index database (`vellum-db`), for
//! both SQLite and Postgres.
//!
//! It reads the tree from `page_ancestors`, keeps tuples in `grants`, and
//! keeps materialised rows in `effective_access`, guarded by
//! `spaces.authz_epoch`. Every statement runs in a `db.query` span.
//!
//! The index stores the space itself in `page_ancestors` (a depth-0 row,
//! and the depth-1 parent of every top-level node); here the space is the
//! [`Object::Space`] instead, so top-level nodes get no parent.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use vellum_db::sqlx::types::Uuid;
use vellum_db::sqlx::{self, QueryBuilder};
use vellum_db::{BeginWrite as _, Index, IndexPool, traced};

use crate::graph::SpaceGraph;
use crate::model::{
    Id, Object, Principal, Relation, Subject, Tuple, TupleRelation,
};
use crate::store::{AuthzStore, Commit, Scope, StoreError, TupleKey};

/// How many effective rows one `INSERT` carries.
const INSERT_BATCH: usize = 500;

const fn uuid(id: Id) -> Uuid {
    Uuid::from_u128(id.0)
}

const fn id(uuid: Uuid) -> Id {
    Id(uuid.as_u128())
}

fn backend(err: impl std::error::Error + Send + Sync + 'static) -> StoreError {
    StoreError::Backend(Box::new(err))
}

fn corrupt(what: &str) -> StoreError {
    StoreError::Backend(format!("unreadable authz row: {what}").into())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn relation(name: &str) -> Result<Relation, StoreError> {
    Relation::parse(name).ok_or_else(|| corrupt(name))
}

fn epoch(value: i64) -> Result<u64, StoreError> {
    u64::try_from(value).map_err(|_| corrupt("negative authz_epoch"))
}

fn signed(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| corrupt("authz_epoch overflow"))
}

/// A `grants` row as read back.
type GrantRow = (String, Uuid, String, String, Uuid, String, Option<i64>);

fn tuple(row: GrantRow) -> Result<Tuple, StoreError> {
    let (
        object_type,
        object_id,
        rel,
        subject_type,
        subject_id,
        subject_rel,
        expires_at,
    ) = row;
    Ok(Tuple {
        object: Object::from_columns(&object_type, id(object_id))
            .ok_or_else(|| corrupt(&object_type))?,
        relation: TupleRelation::parse(&rel).ok_or_else(|| corrupt(&rel))?,
        subject: Subject::from_columns(
            &subject_type,
            id(subject_id),
            &subject_rel,
        )
        .ok_or_else(|| corrupt(&subject_type))?,
        expires_at,
    })
}

const SELECT_GRANTS: &str = "SELECT object_type, object_id, relation, \
     subject_type, subject_id, subject_relation, expires_at FROM grants WHERE \
     space_id = $1";

const UPSERT_GRANT: &str = "INSERT INTO grants (space_id, object_type, \
     object_id, relation, subject_type, subject_id, subject_relation, \
     expires_at, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON \
     CONFLICT (space_id, object_type, object_id, relation, subject_type, \
     subject_id, subject_relation) DO UPDATE SET expires_at = \
     excluded.expires_at";

const DELETE_GRANT: &str = "DELETE FROM grants WHERE space_id = $1 AND \
     object_type = $2 AND object_id = $3 AND relation = $4 AND subject_type = \
     $5 AND subject_id = $6 AND subject_relation = $7";

/// Writes tuple upserts and deletes inside a transaction. A macro because
/// the two pools have different connection types but identical SQL.
macro_rules! write_tuples {
    ($tx:expr, $system:expr, $space:expr, $upsert:expr, $delete:expr) => {{
        let space = uuid($space);
        let now = now_ms();
        for (object, rel, subject) in $delete {
            let (object_type, object_id) = object.columns();
            let (subject_type, subject_id, subject_rel) = subject.columns();
            let q = sqlx::query(DELETE_GRANT)
                .bind(space)
                .bind(object_type)
                .bind(uuid(object_id))
                .bind(rel.as_str())
                .bind(subject_type)
                .bind(uuid(subject_id))
                .bind(subject_rel);
            let _ = traced($system, "authz.delete_grant", q.execute(&mut *$tx))
                .await
                .map_err(backend)?;
        }
        for t in $upsert {
            let (object_type, object_id) = t.object.columns();
            let (subject_type, subject_id, subject_rel) = t.subject.columns();
            let q = sqlx::query(UPSERT_GRANT)
                .bind(space)
                .bind(object_type)
                .bind(uuid(object_id))
                .bind(t.relation.as_str())
                .bind(subject_type)
                .bind(uuid(subject_id))
                .bind(subject_rel)
                .bind(t.expires_at)
                .bind(now);
            let _ = traced($system, "authz.upsert_grant", q.execute(&mut *$tx))
                .await
                .map_err(backend)?;
        }
    }};
}

macro_rules! load_space {
    ($pool:expr, $system:expr, $space:expr) => {{
        let space = uuid($space);
        let row: Option<i64> = traced(
            $system,
            "authz.load_epoch",
            sqlx::query_scalar(
                "SELECT authz_epoch FROM spaces WHERE space_id = $1",
            )
            .bind(space)
            .fetch_optional($pool),
        )
        .await
        .map_err(backend)?;
        let Some(raw_epoch) = row else {
            return Err(StoreError::UnknownSpace($space));
        };
        let links: Vec<(Uuid, Uuid, i32)> = traced(
            $system,
            "authz.load_tree",
            sqlx::query_as(
                "SELECT descendant_id, ancestor_id, depth FROM page_ancestors \
                 WHERE space_id = $1 AND depth <= 1",
            )
            .bind(space)
            .fetch_all($pool),
        )
        .await
        .map_err(backend)?;
        let mut parents: BTreeMap<Id, Option<Id>> = BTreeMap::new();
        for (descendant, ancestor, depth) in links {
            let node = id(descendant);
            if node == $space {
                continue;
            }
            let slot = parents.entry(node).or_insert(None);
            if depth == 1 && id(ancestor) != $space {
                *slot = Some(id(ancestor));
            }
        }
        let rows: Vec<GrantRow> = traced(
            $system,
            "authz.load_grants",
            sqlx::query_as(SELECT_GRANTS).bind(space).fetch_all($pool),
        )
        .await
        .map_err(backend)?;
        Ok(SpaceGraph {
            space: $space,
            epoch: epoch(raw_epoch)?,
            parents,
            tuples: rows.into_iter().map(tuple).collect::<Result<_, _>>()?,
        })
    }};
}

macro_rules! commit {
    ($pool:expr, $system:expr, $db:ty, $commit:expr) => {{
        let c: Commit = $commit;
        let space = uuid(c.space);
        let mut tx = $pool.begin_write().await.map_err(backend)?;
        let bumped = traced(
            $system,
            "authz.bump_epoch",
            sqlx::query(
                "UPDATE spaces SET authz_epoch = authz_epoch + 1 \
                 WHERE space_id = $1 AND authz_epoch = $2",
            )
            .bind(space)
            .bind(signed(c.expected_epoch)?)
            .execute(&mut *tx),
        )
        .await
        .map_err(backend)?;
        if bumped.rows_affected() == 0 {
            let exists: Option<i64> = traced(
                $system,
                "authz.load_epoch",
                sqlx::query_scalar(
                    "SELECT authz_epoch FROM spaces WHERE \
                    space_id = $1",
                )
                .bind(space)
                .fetch_optional(&mut *tx),
            )
            .await
            .map_err(backend)?;
            return Err(match exists {
                Some(_) => StoreError::Conflict,
                None => StoreError::UnknownSpace(c.space),
            });
        }
        let new_epoch = signed(c.expected_epoch + 1)?;
        write_tuples!(tx, $system, c.space, &c.upsert, &c.delete);
        match &c.scope {
            Scope::All => {
                let _ = traced(
                    $system,
                    "authz.clear_rows",
                    sqlx::query(
                        "DELETE FROM effective_access WHERE space_id = \
                        $1",
                    )
                    .bind(space)
                    .execute(&mut *tx),
                )
                .await
                .map_err(backend)?;
            }
            Scope::Objects(objects) => {
                for object in objects {
                    let _ = traced(
                        $system,
                        "authz.clear_object_rows",
                        sqlx::query(
                            "DELETE FROM effective_access WHERE space_id = $1 \
                                AND object_id = $2",
                        )
                        .bind(space)
                        .bind(uuid(*object))
                        .execute(&mut *tx),
                    )
                    .await
                    .map_err(backend)?;
                }
            }
        }
        for chunk in c.rows.chunks(INSERT_BATCH) {
            let mut q: QueryBuilder<$db> = QueryBuilder::new(
                "INSERT INTO effective_access (space_id, subject_id, \
                 object_id, relation, authz_epoch, valid_until) ",
            );
            let _ = q.push_values(chunk, |mut b, row| {
                let _ = b
                    .push_bind(space)
                    .push_bind(uuid(row.principal.id()))
                    .push_bind(uuid(row.object))
                    .push_bind(row.relation.as_str())
                    .push_bind(new_epoch)
                    .push_bind(row.valid_until);
            });
            let _ = traced(
                $system,
                "authz.insert_rows",
                q.build().execute(&mut *tx),
            )
            .await
            .map_err(backend)?;
        }
        tx.commit().await.map_err(backend)?;
        epoch(new_epoch)
    }};
}

macro_rules! lookup_many {
    (
        $pool:expr,
        $system:expr,
        $db:ty,
        $principal:expr,
        $objects:expr,
        $now:expr
    ) => {{
        let mut out = Vec::new();
        for chunk in $objects.chunks(INSERT_BATCH) {
            let mut q: QueryBuilder<$db> = QueryBuilder::new(
                "SELECT object_id, relation FROM effective_access WHERE \
                    subject_id = ",
            );
            let _ = q
                .push_bind(uuid($principal.id()))
                .push(" AND (valid_until IS NULL OR valid_until > ")
                .push_bind($now)
                .push(") AND object_id IN (");
            let mut list = q.separated(", ");
            for object in chunk {
                let _ = list.push_bind(uuid(*object));
            }
            let _ = q.push(")");
            let rows: Vec<(Uuid, String)> = traced(
                $system,
                "authz.lookup_many",
                q.build_query_as().fetch_all($pool),
            )
            .await
            .map_err(backend)?;
            for (object, rel) in rows {
                out.push((id(object), relation(&rel)?));
            }
        }
        Ok(out)
    }};
}

enum Pool<'a> {
    Sqlite(&'a sqlx::SqlitePool),
    Postgres(&'a sqlx::PgPool),
}

/// Authorization tables in the index database.
#[derive(Debug, Clone)]
pub struct SqlStore {
    index: Index,
}

impl SqlStore {
    /// Uses an opened, migrated index.
    #[must_use]
    pub const fn new(index: Index) -> Self {
        Self { index }
    }

    /// The pool, as an exhaustive local enum: a backend this adapter does
    /// not know is an error, not a silent fallback.
    fn pool(&self) -> Result<Pool<'_>, StoreError> {
        match self.index.pool() {
            IndexPool::Sqlite(pool) => Ok(Pool::Sqlite(pool)),
            IndexPool::Postgres(pool) => Ok(Pool::Postgres(pool)),
            _ => Err(StoreError::Backend(
                "this index backend has no authorization adapter".into(),
            )),
        }
    }

    const fn system(&self) -> &'static str {
        self.index.backend().otel_system()
    }
}

const LOOKUP: &str = "SELECT relation, authz_epoch FROM effective_access \
     WHERE space_id = $1 AND subject_id = $2 AND object_id = $3 \
     AND (valid_until IS NULL OR valid_until > $4)";

const LIST: &str = "SELECT object_id, relation FROM effective_access WHERE \
     space_id = $1 AND subject_id = $2 AND (valid_until IS NULL OR valid_until \
     > $3)";

const SPACES_OF: &str = "SELECT DISTINCT space_id FROM effective_access \
     WHERE subject_id = $1 AND (valid_until IS NULL OR valid_until > $2)";

const SPACES_GRANTING: &str = "SELECT DISTINCT space_id FROM grants \
     WHERE subject_type = 'group' AND subject_id = $1 AND space_id <> $2";

impl AuthzStore for SqlStore {
    async fn load_space(&self, space: Id) -> Result<SpaceGraph, StoreError> {
        let system = self.system();
        match self.pool()? {
            Pool::Sqlite(pool) => load_space!(pool, system, space),
            Pool::Postgres(pool) => load_space!(pool, system, space),
        }
    }

    async fn load_directory(&self) -> Result<Vec<Tuple>, StoreError> {
        let system = self.system();
        let rows: Vec<GrantRow> = match self.pool()? {
            Pool::Sqlite(pool) => {
                let q = sqlx::query_as(SELECT_GRANTS).bind(uuid(Id::DIRECTORY));
                traced(system, "authz.load_directory", q.fetch_all(pool)).await
            }
            Pool::Postgres(pool) => {
                let q = sqlx::query_as(SELECT_GRANTS).bind(uuid(Id::DIRECTORY));
                traced(system, "authz.load_directory", q.fetch_all(pool)).await
            }
        }
        .map_err(backend)?;
        rows.into_iter().map(tuple).collect()
    }

    async fn write_directory(
        &self,
        upsert: Vec<Tuple>,
        delete: Vec<TupleKey>,
    ) -> Result<(), StoreError> {
        let system = self.system();
        match self.pool()? {
            Pool::Sqlite(pool) => {
                let mut tx = pool.begin_write().await.map_err(backend)?;
                write_tuples!(tx, system, Id::DIRECTORY, &upsert, &delete);
                tx.commit().await.map_err(backend)
            }
            Pool::Postgres(pool) => {
                let mut tx = pool.begin_write().await.map_err(backend)?;
                write_tuples!(tx, system, Id::DIRECTORY, &upsert, &delete);
                tx.commit().await.map_err(backend)
            }
        }
    }

    async fn spaces_granting(
        &self,
        groups: &BTreeSet<Id>,
    ) -> Result<Vec<Id>, StoreError> {
        let system = self.system();
        let mut out = BTreeSet::new();
        for group in groups {
            let rows: Vec<Uuid> = match self.pool()? {
                Pool::Sqlite(pool) => {
                    let q = sqlx::query_scalar(SPACES_GRANTING)
                        .bind(uuid(*group))
                        .bind(uuid(Id::DIRECTORY));
                    traced(system, "authz.spaces_granting", q.fetch_all(pool))
                        .await
                }
                Pool::Postgres(pool) => {
                    let q = sqlx::query_scalar(SPACES_GRANTING)
                        .bind(uuid(*group))
                        .bind(uuid(Id::DIRECTORY));
                    traced(system, "authz.spaces_granting", q.fetch_all(pool))
                        .await
                }
            }
            .map_err(backend)?;
            out.extend(rows.into_iter().map(id));
        }
        Ok(out.into_iter().collect())
    }

    async fn commit(&self, commit: Commit) -> Result<u64, StoreError> {
        let system = self.system();
        match self.pool()? {
            Pool::Sqlite(pool) => commit!(pool, system, sqlx::Sqlite, commit),
            Pool::Postgres(pool) => {
                commit!(pool, system, sqlx::Postgres, commit)
            }
        }
    }

    async fn lookup(
        &self,
        space: Id,
        principal: Principal,
        object: Id,
        now_ms: i64,
    ) -> Result<Vec<(Relation, u64)>, StoreError> {
        let system = self.system();
        let rows: Vec<(String, i64)> = match self.pool()? {
            Pool::Sqlite(pool) => {
                let q = sqlx::query_as(LOOKUP)
                    .bind(uuid(space))
                    .bind(uuid(principal.id()))
                    .bind(uuid(object))
                    .bind(now_ms);
                traced(system, "authz.lookup", q.fetch_all(pool)).await
            }
            Pool::Postgres(pool) => {
                let q = sqlx::query_as(LOOKUP)
                    .bind(uuid(space))
                    .bind(uuid(principal.id()))
                    .bind(uuid(object))
                    .bind(now_ms);
                traced(system, "authz.lookup", q.fetch_all(pool)).await
            }
        }
        .map_err(backend)?;
        rows.into_iter()
            .map(|(r, e)| Ok((relation(&r)?, epoch(e)?)))
            .collect()
    }

    async fn list(
        &self,
        space: Id,
        principal: Principal,
        now_ms: i64,
    ) -> Result<Vec<(Id, Relation)>, StoreError> {
        let system = self.system();
        let rows: Vec<(Uuid, String)> = match self.pool()? {
            Pool::Sqlite(pool) => {
                let q = sqlx::query_as(LIST)
                    .bind(uuid(space))
                    .bind(uuid(principal.id()))
                    .bind(now_ms);
                traced(system, "authz.list", q.fetch_all(pool)).await
            }
            Pool::Postgres(pool) => {
                let q = sqlx::query_as(LIST)
                    .bind(uuid(space))
                    .bind(uuid(principal.id()))
                    .bind(now_ms);
                traced(system, "authz.list", q.fetch_all(pool)).await
            }
        }
        .map_err(backend)?;
        rows.into_iter()
            .map(|(o, r)| Ok((id(o), relation(&r)?)))
            .collect()
    }

    async fn lookup_many(
        &self,
        principal: Principal,
        objects: &[Id],
        now_ms: i64,
    ) -> Result<Vec<(Id, Relation)>, StoreError> {
        let system = self.system();
        match self.pool()? {
            Pool::Sqlite(pool) => {
                lookup_many!(
                    pool,
                    system,
                    sqlx::Sqlite,
                    principal,
                    objects,
                    now_ms
                )
            }
            Pool::Postgres(pool) => {
                lookup_many!(
                    pool,
                    system,
                    sqlx::Postgres,
                    principal,
                    objects,
                    now_ms
                )
            }
        }
    }

    async fn spaces_of(
        &self,
        principal: Principal,
        now_ms: i64,
    ) -> Result<Vec<Id>, StoreError> {
        let system = self.system();
        let rows: Vec<Uuid> = match self.pool()? {
            Pool::Sqlite(pool) => {
                let q = sqlx::query_scalar(SPACES_OF)
                    .bind(uuid(principal.id()))
                    .bind(now_ms);
                traced(system, "authz.spaces_of", q.fetch_all(pool)).await
            }
            Pool::Postgres(pool) => {
                let q = sqlx::query_scalar(SPACES_OF)
                    .bind(uuid(principal.id()))
                    .bind(now_ms);
                traced(system, "authz.spaces_of", q.fetch_all(pool)).await
            }
        }
        .map_err(backend)?;
        Ok(rows.into_iter().map(id).collect())
    }

    async fn epoch(&self, space: Id) -> Result<u64, StoreError> {
        let system = self.system();
        let row: Option<i64> = match self.pool()? {
            Pool::Sqlite(pool) => {
                let q = sqlx::query_scalar(
                    "SELECT authz_epoch FROM spaces WHERE space_id = $1",
                )
                .bind(uuid(space));
                traced(system, "authz.epoch", q.fetch_optional(pool)).await
            }
            Pool::Postgres(pool) => {
                let q = sqlx::query_scalar(
                    "SELECT authz_epoch FROM spaces WHERE space_id = $1",
                )
                .bind(uuid(space));
                traced(system, "authz.epoch", q.fetch_optional(pool)).await
            }
        }
        .map_err(backend)?;
        row.map_or(Err(StoreError::UnknownSpace(space)), epoch)
    }
}
