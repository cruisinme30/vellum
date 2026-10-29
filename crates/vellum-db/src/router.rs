// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Finding a space's shard at runtime (ADR 0040, D174).
//!
//! [`ShardRouter`] keeps the `space_shards` map in memory. A space the map
//! does not hold yet is looked up in the main database once, so a space
//! another replica placed is found on first use. A new space is placed on
//! the shard holding the fewest spaces; when two replicas place the same
//! space at once, the first entry written wins and both use it.
//!
//! The map is never invalidated: moving a space to another shard is done
//! with every replica stopped (`vellum admin move-space`).

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};

use uuid::Uuid;
use vellum_kernel::ObjectId;

use crate::error::{Error, Result};
use crate::index::{Index, IndexPool};
use crate::shard::ShardedIndex;
use crate::telemetry::traced;

/// Routes each space to the index of the shard that stores it.
#[derive(Debug)]
pub struct ShardRouter {
    sharded: ShardedIndex,
    /// Space to shard position, as far as this replica knows.
    map: RwLock<HashMap<ObjectId, usize>>,
}

impl ShardRouter {
    /// Loads the map from the main database, first recording every space
    /// a shard already holds that the map has no entry for (spaces written
    /// before sharding was turned on, all on the home shard).
    ///
    /// # Errors
    /// [`Error::UnknownShard`] when the map names a shard that is not
    /// configured; database errors.
    pub async fn open(sharded: ShardedIndex) -> Result<Self> {
        if sharded.shards().len() == 1 {
            // One database: every space is at home and the map is unused.
            return Ok(Self {
                sharded,
                map: RwLock::new(HashMap::new()),
            });
        }
        adopt(&sharded).await?;
        let mut map = HashMap::new();
        for (space, name) in sharded.assignments().await? {
            let _ = map.insert(space, position(&sharded, &name)?);
        }
        Ok(Self {
            sharded,
            map: RwLock::new(map),
        })
    }

    /// The main database and every shard.
    #[must_use]
    pub const fn sharded(&self) -> &ShardedIndex {
        &self.sharded
    }

    /// Every shard's index, home first.
    pub fn indexes(&self) -> impl Iterator<Item = &Index> {
        self.sharded.shards().iter().map(crate::shard::Shard::index)
    }

    /// How many shards there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sharded.shards().len()
    }

    /// Always false: a router has at least the home shard.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sharded.shards().is_empty()
    }

    fn known(&self, space: ObjectId) -> Option<usize> {
        self.map
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&space)
            .copied()
    }

    fn learn(&self, space: ObjectId, at: usize) {
        let _ = self
            .map
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(space, at);
    }

    /// The position of the shard storing `space`: its entry in the map,
    /// or the home shard for a space nobody placed.
    ///
    /// # Errors
    /// [`Error::UnknownShard`] or database errors.
    pub async fn position(&self, space: ObjectId) -> Result<usize> {
        if self.len() == 1 {
            return Ok(0);
        }
        if let Some(at) = self.known(space) {
            return Ok(at);
        }
        match self.sharded.assigned(space).await? {
            Some(name) => {
                let at = position(&self.sharded, &name)?;
                self.learn(space, at);
                Ok(at)
            }
            // Not cached: the space may be placed by another replica later.
            None => Ok(0),
        }
    }

    /// The index storing `space`.
    ///
    /// # Errors
    /// As [`Self::position`].
    pub async fn index_for(&self, space: ObjectId) -> Result<&Index> {
        let at = self.position(space).await?;
        self.at(at)
    }

    /// The index at shard position `at`.
    ///
    /// # Errors
    /// [`Error::UnknownShard`] when `at` is out of range.
    pub fn at(&self, at: usize) -> Result<&Index> {
        self.sharded
            .shards()
            .get(at)
            .map(crate::shard::Shard::index)
            .ok_or_else(|| Error::UnknownShard(format!("#{at}")))
    }

    /// Places `space` if the map has no entry for it yet, on the shard
    /// with the fewest spaces, and returns its position.
    ///
    /// # Errors
    /// Database errors.
    pub async fn place(&self, space: ObjectId) -> Result<usize> {
        if self.len() == 1 {
            return Ok(0);
        }
        if let Some(at) = self.known(space) {
            return Ok(at);
        }
        if let Some(name) = self.sharded.assigned(space).await? {
            let at = position(&self.sharded, &name)?;
            self.learn(space, at);
            return Ok(at);
        }
        let counts = self.sharded.counts().await?;
        let shards = self.sharded.shards();
        let pick = shards
            .iter()
            .enumerate()
            .min_by_key(|(i, s)| (counts.get(s.name()).copied().unwrap_or(0), *i))
            .map_or(0, |(i, _)| i);
        let name = shards
            .get(pick)
            .map(|s| s.name().to_owned())
            .unwrap_or_default();
        self.sharded.claim(space, &name).await?;
        // Another replica may have placed it first; its entry stands.
        let winner = self.sharded.assigned(space).await?.unwrap_or(name);
        let at = position(&self.sharded, &winner)?;
        self.learn(space, at);
        Ok(at)
    }
}

/// Records each space found on a shard but missing from the map.
async fn adopt(sharded: &ShardedIndex) -> Result<()> {
    const SQL: &str = "SELECT space_id FROM spaces UNION SELECT space_id FROM drive_spaces";
    let known = sharded.assignments().await?;
    for shard in sharded.shards() {
        let index = shard.index();
        let system = index.backend().otel_system();
        let ids: Vec<Uuid> = match index.pool() {
            IndexPool::Sqlite(pool) => {
                traced(
                    system,
                    "spaces.ids",
                    sqlx::query_scalar(SQL).fetch_all(pool),
                )
                .await?
            }
            IndexPool::Postgres(pool) => {
                traced(
                    system,
                    "spaces.ids",
                    sqlx::query_scalar(SQL).fetch_all(pool),
                )
                .await?
            }
        };
        for id in ids {
            let space = ObjectId::from_uuid(id);
            if !known.contains_key(&space) {
                sharded.claim(space, shard.name()).await?;
            }
        }
    }
    Ok(())
}

fn position(sharded: &ShardedIndex, name: &str) -> Result<usize> {
    sharded
        .shards()
        .iter()
        .position(|s| s.name().eq_ignore_ascii_case(name))
        .ok_or_else(|| Error::UnknownShard(name.to_owned()))
}
