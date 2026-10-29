// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Reading pages and stars from several shards (ADR 0040, D174).
//!
//! Each shard holds whole spaces, so a page is on exactly one shard and a
//! person's stars are the union of every shard's.

use std::collections::HashSet;

use async_trait::async_trait;
use vellum_kernel::{DocId, UserId};

use crate::access::Stars;
use crate::error::Result;
use crate::indexer::DocumentSource;
use crate::model::IndexDocument;

/// Pages from every shard: a load asks each shard in turn, a list joins
/// them.
#[derive(Debug, Clone)]
pub struct Sharded<T>(pub Vec<T>);

#[async_trait]
impl<T: DocumentSource> DocumentSource for Sharded<T> {
    async fn load(&self, doc: DocId) -> Result<Option<IndexDocument>> {
        for shard in &self.0 {
            if let Some(found) = shard.load(doc).await? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }

    async fn list(&self) -> Result<Vec<DocId>> {
        let mut all = Vec::new();
        for shard in &self.0 {
            all.extend(shard.list().await?);
        }
        Ok(all)
    }
}

#[async_trait]
impl<T: Stars> Stars for Sharded<T> {
    async fn starred(&self, user: UserId) -> Result<HashSet<DocId>> {
        let mut all = HashSet::new();
        for shard in &self.0 {
            all.extend(shard.starred(user).await?);
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::conformance::StaticStars;

    struct Docs(Vec<DocId>);

    #[async_trait]
    impl DocumentSource for Docs {
        async fn load(&self, _doc: DocId) -> Result<Option<IndexDocument>> {
            Ok(None)
        }
        async fn list(&self) -> Result<Vec<DocId>> {
            Ok(self.0.clone())
        }
    }

    #[tokio::test]
    async fn lists_and_stars_join_every_shard() {
        let (a, b) = (DocId::from_u128(1), DocId::from_u128(2));
        let docs = Sharded(vec![Docs(vec![a]), Docs(vec![b])]);
        assert_eq!(docs.list().await.unwrap(), [a, b]);
        assert!(docs.load(a).await.unwrap().is_none());

        let user = UserId::from_u128(9);
        let stars = Sharded(vec![
            StaticStars(HashMap::from([(user, HashSet::from([a]))])),
            StaticStars(HashMap::from([(user, HashSet::from([b]))])),
        ]);
        assert_eq!(stars.starred(user).await.unwrap(), HashSet::from([a, b]));
    }
}
