// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The "starred" filter, answered from the index's `user_doc_state` table.
//!
//! Stars are database-owned per-user state (the files do not carry them),
//! written by the drive service when someone stars or unstars an item.
//! Search only reads them, at query time, so a new star counts at once.

use std::collections::HashSet;

use async_trait::async_trait;
use uuid::Uuid;
use vellum_db::{Index, IndexPool, sqlx, traced};
use vellum_kernel::{DocId, UserId};

use crate::access::Stars;
use crate::error::{Result, SearchError};

const STARRED: &str =
    "SELECT doc_id FROM user_doc_state WHERE user_id = $1 AND starred_at IS NOT NULL";

fn failed(error: &sqlx::Error) -> SearchError {
    SearchError::Source(format!("could not read stars: {error}"))
}

#[async_trait]
impl Stars for Index {
    async fn starred(&self, user: UserId) -> Result<HashSet<DocId>> {
        let user = Uuid::from_u128(user.as_u128());
        let rows: Vec<(Uuid,)> = match self.pool() {
            IndexPool::Sqlite(pool) => traced(
                "sqlite",
                "search.starred",
                sqlx::query_as(STARRED).bind(user).fetch_all(pool),
            )
            .await
            .map_err(|e| failed(&e))?,
            IndexPool::Postgres(pool) => traced(
                "postgresql",
                "search.starred",
                sqlx::query_as(STARRED).bind(user).fetch_all(pool),
            )
            .await
            .map_err(|e| failed(&e))?,
            _ => return Err(SearchError::Source("unsupported index backend".into())),
        };
        Ok(rows
            .into_iter()
            .map(|(id,)| DocId::from_u128(id.as_u128()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use vellum_db::IndexConfig;

    use super::*;

    async fn star(index: &Index, user: UserId, doc: DocId, at: Option<i64>) {
        let IndexPool::Sqlite(pool) = index.pool() else {
            panic!("expected SQLite");
        };
        let _ = sqlx::query(
            "INSERT INTO user_doc_state (space_id, user_id, doc_id, starred_at) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::from_u128(1))
        .bind(Uuid::from_u128(user.as_u128()))
        .bind(Uuid::from_u128(doc.as_u128()))
        .bind(at)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn reads_only_the_users_current_stars() {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::open(&IndexConfig::sqlite(dir.path().join("index.db"), None))
            .await
            .unwrap();
        let (ada, grace) = (UserId::from_u128(0x30), UserId::from_u128(0x31));
        let (runbook, budget, issue) = (
            DocId::from_u128(0x40),
            DocId::from_u128(0x41),
            DocId::from_u128(0x42),
        );
        star(&index, ada, runbook, Some(1)).await;
        star(&index, ada, issue, None).await; // viewed, not starred
        star(&index, grace, budget, Some(2)).await;

        assert_eq!(index.starred(ada).await.unwrap(), HashSet::from([runbook]));
        assert_eq!(index.starred(grace).await.unwrap(), HashSet::from([budget]));
        assert!(
            index
                .starred(UserId::from_u128(0x99))
                .await
                .unwrap()
                .is_empty()
        );
    }
}
