// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Authorization tables spread over shards (ADR 0040, D174).
//!
//! A space's tuples, tree and effective rows live on its shard, so every
//! call naming a space goes there. The directory (group memberships) lives
//! on the home shard. Calls across spaces ask every shard and merge.

use std::collections::BTreeSet;
use std::sync::Arc;

use vellum_db::ShardRouter;
use vellum_kernel::ObjectId;

use crate::graph::SpaceGraph;
use crate::model::{Id, Principal, Relation, Tuple};
use crate::sql::SqlStore;
use crate::store::{AuthzStore, Commit, StoreError, TupleKey};

/// The authorization store over every shard of a [`ShardRouter`].
#[derive(Debug, Clone)]
pub struct ShardedStore {
    router: Arc<ShardRouter>,
    /// One adapter per shard, home first.
    shards: Vec<SqlStore>,
}

fn routing(err: vellum_db::Error) -> StoreError {
    StoreError::Backend(Box::new(err))
}

const fn object(space: Id) -> ObjectId {
    ObjectId::from_u128(space.0)
}

impl ShardedStore {
    /// The authorization tables on every shard of `router`.
    #[must_use]
    pub fn new(router: Arc<ShardRouter>) -> Self {
        let shards = router.indexes().cloned().map(SqlStore::new).collect();
        Self { router, shards }
    }

    fn shard(&self, at: usize) -> Result<&SqlStore, StoreError> {
        self.shards.get(at).ok_or_else(|| {
            StoreError::Backend(format!("no shard #{at}").into())
        })
    }

    fn home(&self) -> Result<&SqlStore, StoreError> {
        self.shard(0)
    }

    async fn read(&self, space: Id) -> Result<&SqlStore, StoreError> {
        if space == Id::DIRECTORY {
            return self.home();
        }
        let at = self.router.position(object(space)).await.map_err(routing)?;
        self.shard(at)
    }
}

impl AuthzStore for ShardedStore {
    async fn load_space(&self, space: Id) -> Result<SpaceGraph, StoreError> {
        self.read(space).await?.load_space(space).await
    }

    async fn load_directory(&self) -> Result<Vec<Tuple>, StoreError> {
        self.home()?.load_directory().await
    }

    async fn write_directory(
        &self,
        upsert: Vec<Tuple>,
        delete: Vec<TupleKey>,
    ) -> Result<(), StoreError> {
        self.home()?.write_directory(upsert, delete).await
    }

    async fn spaces_granting(
        &self,
        groups: &BTreeSet<Id>,
    ) -> Result<Vec<Id>, StoreError> {
        let mut all = BTreeSet::new();
        for shard in &self.shards {
            all.extend(shard.spaces_granting(groups).await?);
        }
        Ok(all.into_iter().collect())
    }

    async fn commit(&self, commit: Commit) -> Result<u64, StoreError> {
        // A commit may be the first write anyone makes for a space.
        let shard = if commit.space == Id::DIRECTORY {
            self.home()?
        } else {
            let at = self
                .router
                .place(object(commit.space))
                .await
                .map_err(routing)?;
            self.shard(at)?
        };
        shard.commit(commit).await
    }

    async fn lookup(
        &self,
        space: Id,
        principal: Principal,
        object: Id,
        now_ms: i64,
    ) -> Result<Vec<(Relation, u64)>, StoreError> {
        self.read(space)
            .await?
            .lookup(space, principal, object, now_ms)
            .await
    }

    async fn list(
        &self,
        space: Id,
        principal: Principal,
        now_ms: i64,
    ) -> Result<Vec<(Id, Relation)>, StoreError> {
        self.read(space).await?.list(space, principal, now_ms).await
    }

    async fn lookup_many(
        &self,
        principal: Principal,
        objects: &[Id],
        now_ms: i64,
    ) -> Result<Vec<(Id, Relation)>, StoreError> {
        let mut all = Vec::new();
        for shard in &self.shards {
            all.extend(shard.lookup_many(principal, objects, now_ms).await?);
        }
        Ok(all)
    }

    async fn spaces_of(
        &self,
        principal: Principal,
        now_ms: i64,
    ) -> Result<Vec<Id>, StoreError> {
        let mut all = BTreeSet::new();
        for shard in &self.shards {
            all.extend(shard.spaces_of(principal, now_ms).await?);
        }
        Ok(all.into_iter().collect())
    }

    async fn epoch(&self, space: Id) -> Result<u64, StoreError> {
        self.read(space).await?.epoch(space).await
    }
}
