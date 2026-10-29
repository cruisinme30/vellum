// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Identity records in the index database, shared by every replica
//! (ADR 0037, D160).
//!
//! Each replica keeps a cache of the whole [`State`] and the database
//! version it reflects. Every read first asks the database for rows
//! written since that version (one indexed query, empty when nothing
//! changed), so a session revoked on one replica is gone on the next
//! request to any other. Every write runs in a transaction that bumps
//! `identity_meta.version` first: the row lock serialises writers across
//! replicas, the closure runs on fresh records, and only the records it
//! changed are written, tagged with the new version.
//!
//! The identity API is synchronous, so the database work runs on a
//! dedicated thread with its own runtime and pool; callers wait on a
//! channel (inside `block_in_place` on a multi-threaded runtime). That
//! also keeps the pool off the caller's runtime, so the store works from
//! any runtime, or none.

use std::collections::BTreeMap;
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::sync::{mpsc as tmpsc, oneshot};
use vellum_db::sqlx;
use vellum_db::{Index, IndexConfig, IndexPool};
use vellum_kernel::{Error, ErrorKind};

use crate::store::State;

/// How long a caller waits for the database before giving up.
const WAIT: Duration = Duration::from_secs(10);
/// Tombstones younger than this many versions are kept, so a replica a
/// little behind still sees deletes without a full reload.
const KEEP_TOMBSTONES: i64 = 1024;
/// Old tombstones are purged on every write whose version is a multiple
/// of this.
const PURGE_EVERY: i64 = 64;

/// One changed record: its kind, ID and JSON, or `None` when deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    kind: String,
    id: String,
    record: Option<String>,
}

/// What changed since a version.
#[derive(Debug, Default)]
struct Delta {
    /// The version the delta brings a cache to.
    version: i64,
    /// Whether `rows` is every live record (the cache was too far behind).
    full: bool,
    rows: Vec<Row>,
}

/// An open write: the fresh delta, and where to send the rows to commit.
struct Begun {
    delta: Delta,
    /// The version the commit writes.
    version: i64,
    commit: oneshot::Sender<Vec<Row>>,
    done: mpsc::Receiver<Result<(), Error>>,
}

enum Job {
    Refresh {
        since: i64,
        reply: mpsc::Sender<Result<Delta, Error>>,
    },
    Write {
        since: i64,
        reply: mpsc::Sender<Result<Begun, Error>>,
    },
}

struct Cache {
    state: State,
    version: i64,
}

/// The records in the index database.
pub struct Shared {
    jobs: tmpsc::UnboundedSender<Job>,
    cache: Mutex<Cache>,
    /// Serialises this replica's writers (the database serialises the
    /// rest), so one write holds one connection.
    writing: Mutex<()>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared").finish_non_exhaustive()
    }
}

fn unavailable(why: impl std::fmt::Display) -> Error {
    Error::new(
        ErrorKind::Unavailable,
        format!("the identity database is unavailable: {why}"),
    )
}

fn db(err: sqlx::Error) -> Error {
    unavailable(err)
}

/// Waits for a reply without stalling other tasks on a multi-threaded
/// runtime.
fn wait<T>(rx: &mpsc::Receiver<Result<T, Error>>) -> Result<T, Error> {
    let recv = || match rx.recv_timeout(WAIT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => Err(unavailable("timed out")),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(unavailable("worker stopped")),
    };
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(recv)
        }
        _ => recv(),
    }
}

impl Shared {
    /// Opens (and migrates) the index at `config` on a worker thread and
    /// loads every record.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unavailable`] if the database cannot be opened.
    pub fn open(config: IndexConfig) -> Result<Self, Error> {
        let (jobs, mut queue) = tmpsc::unbounded_channel::<Job>();
        let (opened_tx, opened) = mpsc::channel::<Result<(), Error>>();
        let _worker = std::thread::Builder::new()
            .name("vellum-identity-db".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        let _ = opened_tx.send(Err(unavailable(e)));
                        return;
                    }
                };
                runtime.block_on(async move {
                    let index = match Index::open(&config).await {
                        Ok(index) => index,
                        Err(e) => {
                            let _ = opened_tx.send(Err(unavailable(e)));
                            return;
                        }
                    };
                    let _ = opened_tx.send(Ok(()));
                    // Jobs run as tasks, so reads on different threads
                    // overlap; the loop ends when the store is dropped.
                    while let Some(job) = queue.recv().await {
                        let index = index.clone();
                        drop(tokio::spawn(async move { run(&index, job).await }));
                    }
                    index.close().await;
                });
            })
            .map_err(unavailable)?;
        wait(&opened)?;
        let shared = Self {
            jobs,
            cache: Mutex::new(Cache {
                state: State::default(),
                version: 0,
            }),
            writing: Mutex::new(()),
        };
        shared.refresh()?;
        Ok(shared)
    }

    fn send<T>(&self, job: impl FnOnce(mpsc::Sender<Result<T, Error>>) -> Job) -> Result<T, Error> {
        let (tx, rx) = mpsc::channel();
        self.jobs
            .send(job(tx))
            .map_err(|_| unavailable("worker stopped"))?;
        wait(&rx)
    }

    /// Brings the cache up to date.
    fn refresh(&self) -> Result<(), Error> {
        let since = self.lock().version;
        let delta = self.send(|reply| Job::Refresh { since, reply })?;
        let mut cache = self.lock();
        if delta.version > cache.version || delta.full {
            apply(&mut cache.state, &delta)?;
            cache.version = cache.version.max(delta.version);
        }
        drop(cache);
        Ok(())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads the records as they are in the database now. If the database
    /// cannot be reached, reads the last records seen and logs a warning.
    pub fn read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        if let Err(error) = self.refresh() {
            tracing::warn!(%error, "identity records may be stale");
        }
        f(&self.lock().state)
    }

    /// Changes the records in one transaction. Nothing is kept when the
    /// commit fails.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unavailable`] if the database cannot be written.
    pub fn write<R>(&self, f: impl FnOnce(&mut State) -> R) -> Result<R, Error> {
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let since = self.lock().version;
        let begun = self.send(|reply| Job::Write { since, reply })?;
        // No other writer can commit until this one ends, so the cache plus
        // the delta is exactly the database at `begun.version - 1`.
        let mut base = self.lock().state.clone();
        apply(&mut base, &begun.delta)?;
        let mut next = base.clone();
        let result = f(&mut next);
        let rows = diff(&base, &next)?;
        let changed = !rows.is_empty();
        let _ = begun.commit.send(rows);
        wait(&begun.done)?;
        let mut cache = self.lock();
        if changed {
            cache.state = next;
            cache.version = begun.version;
        } else if cache.version < begun.version - 1 {
            cache.state = base;
            cache.version = begun.version - 1;
        }
        drop(cache);
        Ok(result)
    }
}

async fn run(index: &Index, job: Job) {
    match job {
        Job::Refresh { since, reply } => {
            let _ = reply.send(refresh(index, since).await);
        }
        Job::Write { since, reply } => {
            let (commit, rows) = oneshot::channel();
            let (done_tx, done) = mpsc::channel();
            let begin = |delta, version| Begun {
                delta,
                version,
                commit,
                done,
            };
            write(index, since, begin, reply, rows, done_tx).await;
        }
    }
}

/// Runs the same statements on whichever pool the index has. SQLite and
/// Postgres both take `$n` placeholders in sqlx.
macro_rules! on_pool {
    ($index:expr, |$pool:ident, $postgres:ident| $body:expr) => {
        match $index.pool() {
            IndexPool::Sqlite($pool) => {
                let $postgres = false;
                $body
            }
            IndexPool::Postgres($pool) => {
                let $postgres = true;
                $body
            }
            _ => Err(unavailable("unsupported database")),
        }
    };
}

type RawRow = (String, String, Option<String>);

async fn refresh(index: &Index, since: i64) -> Result<Delta, Error> {
    on_pool!(index, |pool, postgres| {
        let mut tx = pool.begin().await.map_err(db)?;
        if postgres {
            // One snapshot for the version and the rows.
            let _ = sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
                .execute(&mut *tx)
                .await
                .map_err(db)?;
        }
        let (version, horizon): (i64, i64) =
            sqlx::query_as("SELECT version, horizon FROM identity_meta WHERE id = 1")
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        let delta = if version == since {
            Delta {
                version,
                full: false,
                rows: Vec::new(),
            }
        } else if since < horizon || since > version {
            // Too far behind (deletes were purged), or a rebuilt database.
            let rows: Vec<RawRow> = sqlx::query_as(
                "SELECT kind, id, record FROM identity_records WHERE record IS NOT NULL",
            )
            .fetch_all(&mut *tx)
            .await
            .map_err(db)?;
            Delta {
                version,
                full: true,
                rows: rows.into_iter().map(into_row).collect(),
            }
        } else {
            let rows: Vec<RawRow> =
                sqlx::query_as("SELECT kind, id, record FROM identity_records WHERE version > $1")
                    .bind(since)
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(db)?;
            Delta {
                version,
                full: false,
                rows: rows.into_iter().map(into_row).collect(),
            }
        };
        tx.rollback().await.map_err(db)?;
        Ok(delta)
    })
}

fn into_row((kind, id, record): RawRow) -> Row {
    Row { kind, id, record }
}

async fn write(
    index: &Index,
    since: i64,
    begin: impl FnOnce(Delta, i64) -> Begun,
    reply: mpsc::Sender<Result<Begun, Error>>,
    rows: oneshot::Receiver<Vec<Row>>,
    done: mpsc::Sender<Result<(), Error>>,
) {
    let result: Result<(), Error> = on_pool!(index, |pool, _postgres| {
        let opened = async {
            let mut tx = pool.begin().await.map_err(db)?;
            // Bumping the version takes the row lock that serialises
            // writers on every replica.
            let (version, horizon): (i64, i64) = sqlx::query_as(
                "UPDATE identity_meta SET version = version + 1 WHERE id = 1 \
                 RETURNING version, horizon",
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
            let previous = version - 1;
            let delta = if since == previous {
                Delta {
                    version: previous,
                    full: false,
                    rows: Vec::new(),
                }
            } else if since < horizon || since > previous {
                let rows: Vec<RawRow> = sqlx::query_as(
                    "SELECT kind, id, record FROM identity_records WHERE record IS NOT NULL",
                )
                .fetch_all(&mut *tx)
                .await
                .map_err(db)?;
                Delta {
                    version: previous,
                    full: true,
                    rows: rows.into_iter().map(into_row).collect(),
                }
            } else {
                let rows: Vec<RawRow> = sqlx::query_as(
                    "SELECT kind, id, record FROM identity_records WHERE version > $1",
                )
                .bind(since)
                .fetch_all(&mut *tx)
                .await
                .map_err(db)?;
                Delta {
                    version: previous,
                    full: false,
                    rows: rows.into_iter().map(into_row).collect(),
                }
            };
            Ok::<_, Error>((tx, version, horizon, delta))
        };
        match opened.await {
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
            Ok((mut tx, version, horizon, delta)) => {
                if reply.send(Ok(begin(delta, version))).is_err() {
                    return;
                }
                // The caller dropped the write (it panicked or timed out).
                let Ok(rows) = rows.await else { return };
                if rows.is_empty() {
                    // Nothing changed: leave the version where it was.
                    tx.rollback().await.map_err(db)
                } else {
                    async {
                        for row in &rows {
                            let _ = sqlx::query(
                                "INSERT INTO identity_records (kind, id, version, record) \
                                 VALUES ($1, $2, $3, $4) ON CONFLICT (kind, id) DO UPDATE \
                                 SET version = excluded.version, record = excluded.record",
                            )
                            .bind(&row.kind)
                            .bind(&row.id)
                            .bind(version)
                            .bind(row.record.as_deref())
                            .execute(&mut *tx)
                            .await
                            .map_err(db)?;
                        }
                        let purge_to = version - KEEP_TOMBSTONES;
                        if version % PURGE_EVERY == 0 && purge_to > horizon {
                            let _ = sqlx::query(
                                "DELETE FROM identity_records \
                                 WHERE record IS NULL AND version <= $1",
                            )
                            .bind(purge_to)
                            .execute(&mut *tx)
                            .await
                            .map_err(db)?;
                            let _ =
                                sqlx::query("UPDATE identity_meta SET horizon = $1 WHERE id = 1")
                                    .bind(purge_to)
                                    .execute(&mut *tx)
                                    .await
                                    .map_err(db)?;
                        }
                        tx.commit().await.map_err(db)
                    }
                    .await
                }
            }
        }
    });
    let _ = done.send(result);
}

fn encode<T: Serialize>(value: &T) -> Result<String, Error> {
    serde_json::to_string(value)
        .map_err(|e| Error::new(ErrorKind::Internal, format!("encode identity record: {e}")))
}

fn decode<T: DeserializeOwned>(kind: &str, id: &str, json: &str) -> Result<T, Error> {
    serde_json::from_str(json).map_err(|e| {
        Error::new(
            ErrorKind::Internal,
            format!("cannot parse identity record {kind}/{id}: {e}"),
        )
    })
}

/// Rows for every record that differs between `old` and `new` in one map.
fn diff_map<K: Ord, V: PartialEq + Serialize>(
    kind: &str,
    old: &BTreeMap<K, V>,
    new: &BTreeMap<K, V>,
    id: impl Fn(&K) -> String,
    rows: &mut Vec<Row>,
) -> Result<(), Error> {
    for (key, value) in new {
        if old.get(key) != Some(value) {
            rows.push(Row {
                kind: kind.to_owned(),
                id: id(key),
                record: Some(encode(value)?),
            });
        }
    }
    for key in old.keys().filter(|k| !new.contains_key(*k)) {
        rows.push(Row {
            kind: kind.to_owned(),
            id: id(key),
            record: None,
        });
    }
    Ok(())
}

/// The rows that turn `old` into `new`.
pub fn diff(old: &State, new: &State) -> Result<Vec<Row>, Error> {
    let mut rows = Vec::new();
    diff_map(
        "user",
        &old.users,
        &new.users,
        ToString::to_string,
        &mut rows,
    )?;
    diff_map(
        "session",
        &old.sessions,
        &new.sessions,
        Clone::clone,
        &mut rows,
    )?;
    diff_map(
        "api_token",
        &old.api_tokens,
        &new.api_tokens,
        Clone::clone,
        &mut rows,
    )?;
    diff_map("grant", &old.grants, &new.grants, Clone::clone, &mut rows)?;
    diff_map(
        "access_token",
        &old.access_tokens,
        &new.access_tokens,
        Clone::clone,
        &mut rows,
    )?;
    diff_map(
        "lockout",
        &old.lockouts,
        &new.lockouts,
        Clone::clone,
        &mut rows,
    )?;
    diff_map(
        "oauth_consent",
        &old.oauth_consents,
        &new.oauth_consents,
        Clone::clone,
        &mut rows,
    )?;
    diff_map(
        "oauth_code",
        &old.oauth_codes,
        &new.oauth_codes,
        Clone::clone,
        &mut rows,
    )?;
    Ok(rows)
}

fn put<V: DeserializeOwned>(map: &mut BTreeMap<String, V>, row: &Row) -> Result<(), Error> {
    match &row.record {
        Some(json) => {
            let _ = map.insert(row.id.clone(), decode(&row.kind, &row.id, json)?);
        }
        None => {
            let _ = map.remove(&row.id);
        }
    }
    Ok(())
}

/// Applies rows to a state. Rows of unknown kinds (written by a newer
/// replica during a rolling upgrade) are skipped.
fn apply(state: &mut State, delta: &Delta) -> Result<(), Error> {
    if delta.full {
        *state = State::default();
    }
    for row in &delta.rows {
        match row.kind.as_str() {
            "user" => {
                let id = row.id.parse().map_err(|_| {
                    Error::new(
                        ErrorKind::Internal,
                        format!("bad user ID in the identity records: {}", row.id),
                    )
                })?;
                match &row.record {
                    Some(json) => {
                        let _ = state.users.insert(id, decode(&row.kind, &row.id, json)?);
                    }
                    None => {
                        let _ = state.users.remove(&id);
                    }
                }
            }
            "session" => put(&mut state.sessions, row)?,
            "api_token" => put(&mut state.api_tokens, row)?,
            "grant" => put(&mut state.grants, row)?,
            "access_token" => put(&mut state.access_tokens, row)?,
            "lockout" => put(&mut state.lockouts, row)?,
            "oauth_consent" => put(&mut state.oauth_consents, row)?,
            "oauth_code" => put(&mut state.oauth_codes, row)?,
            other => tracing::debug!(kind = other, "skipping an unknown identity record"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use vellum_kernel::{Timestamp, UserId};

    use super::*;
    use crate::store::{Session, SessionMethod, User};

    fn sample() -> State {
        let mut state = State::default();
        let id = UserId::generate();
        let _ = state.users.insert(
            id,
            User {
                id,
                email: "priya@northstar.dev".into(),
                name: "Priya".into(),
                handle: "priya".into(),
                password_hash: None,
                admin: true,
                disabled: false,
                created_at: Timestamp::now(),
            },
        );
        let _ = state.sessions.insert(
            "s1".into(),
            Session {
                id: "s1".into(),
                hash: [7; 32],
                user: id,
                method: SessionMethod::Password,
                created_at: Timestamp::now(),
                last_seen_at: Timestamp::now(),
                authenticated_at: Timestamp::now(),
                expires_at: Timestamp::now(),
                user_agent: None,
            },
        );
        state
    }

    #[test]
    fn a_diff_replays_to_the_same_state() {
        let old = sample();
        let mut new = old.clone();
        let _ = new.sessions.remove("s1");
        if let Some(user) = new.users.values_mut().next() {
            user.name = "Priya S".into();
        }
        let rows = diff(&old, &new).unwrap();
        assert_eq!(rows.len(), 2);
        let mut replayed = old;
        apply(
            &mut replayed,
            &Delta {
                version: 2,
                full: false,
                rows,
            },
        )
        .unwrap();
        assert_eq!(replayed, new);
        assert!(diff(&new, &new).unwrap().is_empty());
    }

    #[test]
    fn every_state_map_has_a_kind() {
        // A new map in State must be stored too: add it to diff and apply,
        // or it silently stays on one replica.
        let json = serde_json::to_value(State::default()).unwrap();
        let mut fields: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        fields.sort_unstable();
        let mut maps = vec![
            "users",
            "sessions",
            "api_tokens",
            "grants",
            "access_tokens",
            "lockouts",
            "oauth_consents",
            "oauth_codes",
        ];
        maps.sort_unstable();
        assert_eq!(fields, maps);
    }
}
