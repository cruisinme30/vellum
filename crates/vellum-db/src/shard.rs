// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Spreading spaces over several databases.
//!
//! The deploy contract:
//! - `VELLUM_DATABASE_URL` is the main database. It holds the `space_shards`
//!   map from each space to the shard that stores it.
//! - `VELLUM_SHARDS` lists shard names, comma-separated. The first is the
//!   home shard, where a space with no entry in the map lives.
//! - `VELLUM_SHARD_<NAME>_DATABASE_URL` is each shard's database, with the
//!   name in upper case.
//!
//! Every shard carries the full schema. A space's spaces, folders,
//! documents, closure, grants and per-user rows all live on its shard,
//! because `space_id` is part of every key.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tracing::Instrument as _;
use uuid::Uuid;
use vellum_kernel::ObjectId;

use crate::config::IndexConfig;
use crate::error::{Error, Result};
use crate::index::{Index, Pool};
use crate::reindex::{self, FileReader, ReindexReport, Snapshot};
use crate::telemetry::traced;

/// Environment variable naming the main database.
pub const MAIN_URL_VAR: &str = "VELLUM_DATABASE_URL";
/// Environment variable listing the shard names; the first is home.
pub const SHARDS_VAR: &str = "VELLUM_SHARDS";

/// The environment variable holding shard `name`'s database URL.
#[must_use]
pub fn shard_url_var(name: &str) -> String {
    format!("VELLUM_SHARD_{}_DATABASE_URL", name.to_ascii_uppercase())
}

/// Where the main database and each shard live.
#[derive(Debug, Clone)]
pub struct ShardsConfig {
    /// The main database, which holds the space-to-shard map.
    pub main: IndexConfig,
    /// Shard names (lower case) and databases. The first is the home shard.
    pub shards: Vec<(String, IndexConfig)>,
}

impl ShardsConfig {
    /// Reads the deploy contract through `var`, normally `std::env::var`.
    /// Returns `Ok(None)` when `VELLUM_SHARDS` is unset or blank: the
    /// deployment then has a single database.
    ///
    /// # Errors
    /// [`Error::Config`] when the main URL or a shard URL is missing, a
    /// name is empty, repeated or not `[A-Za-z0-9_]`, or two shards share
    /// a database.
    pub fn from_env(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>> {
        let Some(list) = var(SHARDS_VAR).filter(|v| !v.trim().is_empty()) else {
            return Ok(None);
        };
        let main = var(MAIN_URL_VAR)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| {
                Error::Config(format!("{SHARDS_VAR} is set but {MAIN_URL_VAR} is not"))
            })?;
        let mut names = HashSet::new();
        let mut urls = HashSet::new();
        let mut shards = Vec::new();
        for raw in list.split(',') {
            let name = raw.trim().to_ascii_lowercase();
            if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
                return Err(Error::Config(format!(
                    "shard name {raw:?} in {SHARDS_VAR} must be letters, digits or _"
                )));
            }
            if !names.insert(name.clone()) {
                return Err(Error::Config(format!("shard {name:?} is listed twice")));
            }
            let key = shard_url_var(&name);
            let url = var(&key)
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| Error::Config(format!("shard {name:?} needs {key}")))?;
            if !urls.insert(url.clone()) {
                return Err(Error::Config(format!(
                    "shard {name:?} uses the same database as another shard"
                )));
            }
            shards.push((name, IndexConfig::postgres(url)));
        }
        Ok(Some(Self {
            main: IndexConfig::postgres(main),
            shards,
        }))
    }
}

/// One shard: its name and its index.
#[derive(Debug, Clone)]
pub struct Shard {
    name: String,
    index: Index,
}

impl Shard {
    /// The shard's name, lower case.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The shard's index.
    #[must_use]
    pub const fn index(&self) -> &Index {
        &self.index
    }
}

/// The main database plus every shard, all migrated. Cheap to clone.
#[derive(Debug, Clone)]
pub struct ShardedIndex {
    main: Index,
    /// A copy of the first shard.
    home: Shard,
    /// Every shard, home first.
    shards: Vec<Shard>,
}

impl ShardedIndex {
    /// Opens and migrates the main database and every shard.
    ///
    /// # Errors
    /// [`Error::Config`] when no shard is listed; otherwise any error from
    /// [`Index::open`].
    #[tracing::instrument(name = "db.open_shards", skip_all, fields(shards = config.shards.len()))]
    pub async fn open(config: &ShardsConfig) -> Result<Self> {
        let main = Index::open(&config.main).await?;
        let mut shards = Vec::with_capacity(config.shards.len());
        for (name, shard) in &config.shards {
            let index = Index::open(shard)
                .instrument(tracing::info_span!("db.shard", db.shard = %name))
                .await?;
            shards.push(Shard {
                name: name.clone(),
                index,
            });
        }
        let home = shards
            .first()
            .cloned()
            .ok_or_else(|| Error::Config("at least one shard is needed".into()))?;
        Ok(Self { main, home, shards })
    }

    /// One database acting as both the main database and the only shard,
    /// `home`: a deployment without `VELLUM_SHARDS` routes through the same
    /// code with every space at home.
    #[must_use]
    pub fn single(index: Index) -> Self {
        let home = Shard {
            name: "home".to_owned(),
            index: index.clone(),
        };
        Self {
            main: index,
            home: home.clone(),
            shards: vec![home],
        }
    }

    /// The main database.
    #[must_use]
    pub const fn main(&self) -> &Index {
        &self.main
    }

    /// Every shard, home first.
    #[must_use]
    pub fn shards(&self) -> &[Shard] {
        &self.shards
    }

    /// The home shard, where unassigned spaces live.
    #[must_use]
    pub const fn home(&self) -> &Shard {
        &self.home
    }

    /// The shard called `name` (any case).
    #[must_use]
    pub fn shard(&self, name: &str) -> Option<&Shard> {
        self.shards
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// The shard that stores `space`: its entry in the map, or home.
    ///
    /// # Errors
    /// [`Error::UnknownShard`] when the map names a shard that is not
    /// configured; database errors from the main database.
    #[tracing::instrument(name = "db.shard_for", skip_all, fields(db.shard))]
    pub async fn shard_for(&self, space: ObjectId) -> Result<&Shard> {
        let shard = match self.assigned(space).await? {
            Some(name) => self.shard(&name).ok_or(Error::UnknownShard(name))?,
            None => self.home(),
        };
        let _ = tracing::Span::current().record("db.shard", shard.name.as_str());
        Ok(shard)
    }

    /// The shard named for `space` in the map, if any.
    ///
    /// # Errors
    /// Database errors from the main database.
    pub async fn assigned(&self, space: ObjectId) -> Result<Option<String>> {
        let id = space.as_uuid();
        let system = self.main.backend().otel_system();
        let sql = "SELECT shard FROM space_shards WHERE space_id = $1";
        Ok(match &self.main.pool {
            Pool::Sqlite(pool) => {
                let q = sqlx::query_scalar(sql).bind(id).fetch_optional(pool);
                traced(system, "space_shards.get", q).await?
            }
            Pool::Postgres(pool) => {
                let q = sqlx::query_scalar(sql).bind(id).fetch_optional(pool);
                traced(system, "space_shards.get", q).await?
            }
        })
    }

    /// How many spaces the map puts on each shard, by name.
    ///
    /// # Errors
    /// Database errors from the main database.
    pub async fn counts(&self) -> Result<HashMap<String, u64>> {
        let system = self.main.backend().otel_system();
        let sql = "SELECT shard, COUNT(*) FROM space_shards GROUP BY shard";
        let rows: Vec<(String, i64)> = match &self.main.pool {
            Pool::Sqlite(pool) => {
                let q = sqlx::query_as(sql).fetch_all(pool);
                traced(system, "space_shards.count", q).await?
            }
            Pool::Postgres(pool) => {
                let q = sqlx::query_as(sql).fetch_all(pool);
                traced(system, "space_shards.count", q).await?
            }
        };
        Ok(rows
            .into_iter()
            .map(|(name, n)| (name.to_ascii_lowercase(), u64::try_from(n).unwrap_or(0)))
            .collect())
    }

    /// Records `space` on shard `name` unless the map already has an
    /// entry for it, which then stands.
    ///
    /// # Errors
    /// Database errors from the main database.
    pub async fn claim(&self, space: ObjectId, name: &str) -> Result<()> {
        self.record(&[(space, name.to_owned())], false).await
    }

    /// Records that `space` lives on shard `name`. This only updates the
    /// map; run [`Self::reindex`] afterwards to move the space's rows.
    ///
    /// # Errors
    /// [`Error::UnknownShard`] for a name that is not configured; database
    /// errors from the main database.
    #[tracing::instrument(name = "db.assign_shard", skip_all, fields(db.shard = name))]
    pub async fn assign(&self, space: ObjectId, name: &str) -> Result<()> {
        let shard = self
            .shard(name)
            .ok_or_else(|| Error::UnknownShard(name.to_owned()))?;
        let assignments = [(space, shard.name.clone())];
        self.record(&assignments, true).await
    }

    /// Rebuilds every shard from the workspace at `root`. The files are read
    /// once; each shard then gets its own spaces in one transaction per
    /// shard. Spaces not yet in the map are recorded on the home shard. A
    /// space moved with [`Self::assign`] is written to its new shard and
    /// dropped from the old one.
    ///
    /// Each shard commits on its own: if one fails, shards written before
    /// it keep the new index and the rest keep the old one. Running the
    /// reindex again converges.
    ///
    /// # Errors
    /// [`Error::UnknownShard`] before anything is written, when the map
    /// names a shard that is not configured; otherwise scan or database
    /// errors.
    #[tracing::instrument(name = "db.reindex_shards", skip_all, fields(root = %root.display(), documents))]
    pub async fn reindex<R>(&self, root: &Path, reader: R) -> Result<ReindexReport>
    where
        R: FileReader + 'static,
    {
        let now = reindex::now_ms();
        let snap = reindex::scan(root, reader, now).await?;
        let placement = self.place(&snap).await?;
        for (i, shard) in self.shards.iter().enumerate() {
            let part = snap.split_off(|space| placement.get(&space) == Some(&i), false);
            shard
                .index
                .write_snapshot(&part, now)
                .instrument(tracing::info_span!("db.shard", db.shard = %shard.name))
                .await?;
        }
        let report = snap.split_off(|_| true, true).report();
        let _ = tracing::Span::current().record("documents", report.documents);
        Ok(report)
    }

    /// Maps each scanned space to a shard position, recording new spaces
    /// on the home shard.
    async fn place(&self, snap: &Snapshot) -> Result<HashMap<ObjectId, usize>> {
        let known = self.assignments().await?;
        let mut placement = HashMap::new();
        let mut new = Vec::new();
        for space in snap.space_ids() {
            let position = if let Some(name) = known.get(&space) {
                self.shards
                    .iter()
                    .position(|s| s.name.eq_ignore_ascii_case(name))
                    .ok_or_else(|| Error::UnknownShard(name.clone()))?
            } else {
                new.push((space, self.home.name.clone()));
                0
            };
            let _ = placement.insert(space, position);
        }
        self.record(&new, false).await?;
        Ok(placement)
    }

    /// Every entry of the map.
    pub(crate) async fn assignments(&self) -> Result<HashMap<ObjectId, String>> {
        let system = self.main.backend().otel_system();
        let sql = "SELECT space_id, shard FROM space_shards";
        let rows: Vec<(Uuid, String)> = match &self.main.pool {
            Pool::Sqlite(pool) => {
                traced(
                    system,
                    "space_shards.list",
                    sqlx::query_as(sql).fetch_all(pool),
                )
                .await?
            }
            Pool::Postgres(pool) => {
                traced(
                    system,
                    "space_shards.list",
                    sqlx::query_as(sql).fetch_all(pool),
                )
                .await?
            }
        };
        Ok(rows
            .into_iter()
            .map(|(id, name)| (ObjectId::from_uuid(id), name))
            .collect())
    }

    /// Writes map entries in one transaction. `replace` overwrites an
    /// existing entry; otherwise an existing entry wins.
    async fn record(&self, entries: &[(ObjectId, String)], replace: bool) -> Result<()> {
        if entries.is_empty() {
            return Ok(());
        }
        let now = reindex::now_ms();
        let system = self.main.backend().otel_system();
        let sql = if replace {
            "INSERT INTO space_shards (space_id, shard, assigned_at) VALUES ($1, $2, $3) \
             ON CONFLICT (space_id) DO UPDATE SET shard = excluded.shard, \
             assigned_at = excluded.assigned_at"
        } else {
            "INSERT INTO space_shards (space_id, shard, assigned_at) VALUES ($1, $2, $3) \
             ON CONFLICT (space_id) DO NOTHING"
        };
        macro_rules! write {
            ($pool:expr) => {{
                let mut tx = $pool.begin().await?;
                for (space, name) in entries {
                    let q = sqlx::query(sql).bind(space.as_uuid()).bind(name).bind(now);
                    let _ = traced(system, "space_shards.put", q.execute(&mut *tx)).await?;
                }
                tx.commit().await?;
            }};
        }
        match &self.main.pool {
            Pool::Sqlite(pool) => write!(pool),
            Pool::Postgres(pool) => write!(pool),
        }
        Ok(())
    }

    /// Closes the main database and every shard.
    pub async fn close(&self) {
        self.main.close().await;
        for shard in &self.shards {
            shard.index.close().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{ShardsConfig, shard_url_var};
    use crate::error::Error;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key| map.get(key).cloned()
    }

    fn config_error(pairs: &[(&str, &str)]) -> String {
        match ShardsConfig::from_env(env(pairs)) {
            Err(Error::Config(why)) => why,
            other => panic!("expected a configuration error, got {other:?}"),
        }
    }

    #[test]
    fn unset_or_blank_means_one_database() {
        assert!(ShardsConfig::from_env(env(&[])).unwrap().is_none());
        assert!(
            ShardsConfig::from_env(env(&[("VELLUM_SHARDS", " ")]))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reads_names_in_order_with_home_first() {
        let config = ShardsConfig::from_env(env(&[
            ("VELLUM_DATABASE_URL", "postgres://main"),
            ("VELLUM_SHARDS", "eu, US_east"),
            ("VELLUM_SHARD_EU_DATABASE_URL", "postgres://eu"),
            ("VELLUM_SHARD_US_EAST_DATABASE_URL", "postgres://us"),
        ]))
        .unwrap()
        .unwrap();
        let names: Vec<&str> = config.shards.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["eu", "us_east"]);
        assert_eq!(
            shard_url_var("us_east"),
            "VELLUM_SHARD_US_EAST_DATABASE_URL"
        );
    }

    #[test]
    fn rejects_inconsistent_settings() {
        assert!(config_error(&[("VELLUM_SHARDS", "eu")]).contains("VELLUM_DATABASE_URL"));
        let main = ("VELLUM_DATABASE_URL", "postgres://main");
        assert!(
            config_error(&[main, ("VELLUM_SHARDS", "eu")]).contains("VELLUM_SHARD_EU_DATABASE_URL")
        );
        assert!(config_error(&[main, ("VELLUM_SHARDS", ",eu")]).contains("letters"));
        assert!(config_error(&[main, ("VELLUM_SHARDS", "e-u")]).contains("letters"));
        assert!(
            config_error(&[
                main,
                ("VELLUM_SHARDS", "eu,EU"),
                ("VELLUM_SHARD_EU_DATABASE_URL", "postgres://eu"),
            ])
            .contains("twice")
        );
        assert!(
            config_error(&[
                main,
                ("VELLUM_SHARDS", "eu,us"),
                ("VELLUM_SHARD_EU_DATABASE_URL", "postgres://x"),
                ("VELLUM_SHARD_US_DATABASE_URL", "postgres://x"),
            ])
            .contains("same database")
        );
    }
}
