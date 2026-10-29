// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Search's read checks, answered by `vellum-authz`.
//!
//! [`Authz::spaces`] narrows a query to the spaces the principal can read
//! anything in, and [`Authz::filter`] is the exact batch check on the
//! candidate documents. Both read the materialised `effective_access`
//! table, so a grant or revocation reaches search as soon as it commits.

use std::collections::HashSet;

use async_trait::async_trait;
use vellum_authz::{Authz, AuthzStore, Id, Permission};
use vellum_kernel::{DocId, SpaceId};

use crate::access::{Principal, ReadAccess};
use crate::error::{Result, SearchError};

fn principal(principal: Principal) -> vellum_authz::Principal {
    match principal {
        Principal::User(user) => vellum_authz::Principal::from(user),
        Principal::ShareLink(share) => vellum_authz::Principal::from(share),
    }
}

fn failed(error: &vellum_authz::Error) -> SearchError {
    SearchError::Access(error.to_string())
}

#[async_trait]
impl<S: AuthzStore + 'static> ReadAccess for Authz<S> {
    async fn readable_spaces(&self, who: &Principal) -> Result<Vec<SpaceId>> {
        let spaces = self.spaces(principal(*who)).await.map_err(|e| failed(&e))?;
        Ok(spaces.into_iter().map(SpaceId::from).collect())
    }

    async fn filter_readable(&self, who: &Principal, docs: &[DocId]) -> Result<HashSet<DocId>> {
        if docs.is_empty() {
            return Ok(HashSet::new());
        }
        let ids: Vec<Id> = docs.iter().copied().map(Id::from).collect();
        let allowed = self
            .filter(principal(*who), Permission::Read, &ids)
            .await
            .map_err(|e| failed(&e))?;
        Ok(allowed.into_iter().map(DocId::from).collect())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use vellum_authz::{Actor, Change, MemoryStore, Object, Relation, Subject, Tuple};
    use vellum_kernel::{ShareId, UserId};

    use super::*;
    use crate::conformance::{Corpus, StaticStars};
    use crate::model::SearchQuery;
    use crate::service::{SearchService, Update};
    use crate::tantivy_index::{Location, TantivyBackend, TantivyConfig};

    /// The conformance corpus with its spaces and folder trees loaded into
    /// an in-memory authorization store, and nobody granted anything.
    fn authz(corpus: &Corpus) -> Arc<Authz<MemoryStore>> {
        let store = MemoryStore::new();
        let mut trees: BTreeMap<SpaceId, BTreeMap<Id, Option<Id>>> = BTreeMap::new();
        for doc in corpus.documents() {
            let tree = trees.entry(doc.space).or_default();
            let mut parent = None;
            let space = doc.space.as_u128();
            for ancestor in doc.ancestors.iter().filter(|a| a.as_u128() != space) {
                let id = Id(ancestor.as_u128());
                let _ = tree.insert(id, parent);
                parent = Some(id);
            }
            let _ = tree.insert(Id::from(doc.doc), parent);
        }
        for (space, tree) in trees {
            store.set_tree(Id::from(space), tree).unwrap();
        }
        Arc::new(Authz::new(store))
    }

    async fn service(corpus: &Corpus, access: Arc<Authz<MemoryStore>>) -> SearchService {
        let (backend, _) = TantivyBackend::open(&TantivyConfig {
            location: Location::Memory,
            key: None,
            model: None,
        })
        .unwrap();
        let service =
            SearchService::new(Arc::new(backend), access, Arc::new(StaticStars::default()));
        service
            .write(corpus.documents().into_iter().map(Update::Upsert).collect())
            .await
            .unwrap();
        service
    }

    fn grant(object: Object, relation: Relation, user: UserId) -> Change {
        Change::Grant(Tuple::role(object, relation, Subject::User(user.into())))
    }

    #[tokio::test]
    async fn search_sees_grants_and_revocations_at_once() {
        let corpus = Corpus::default();
        let authz = authz(&corpus);
        let search = service(&corpus, Arc::clone(&authz)).await;
        let ada = Principal::User(corpus.ada);
        let query = SearchQuery::new("gateway", 20);

        // Deny by default: nothing granted, nothing found.
        assert!(search.search(&ada, &query).await.unwrap().hits.is_empty());

        // A grant on one document makes exactly that document searchable.
        let space = Id::from(corpus.eng);
        let doc = Id::from(corpus.runbook);
        let _ = authz
            .write(
                space,
                Actor::System,
                &[grant(Object::Node(doc), Relation::Viewer, corpus.ada)],
            )
            .await
            .unwrap();
        let page = search.search(&ada, &query).await.unwrap();
        assert!(!page.hits.is_empty());
        assert!(page.hits.iter().all(|hit| hit.doc == corpus.runbook));

        // Revoking it hides it again, with no reindex.
        let revoke = Change::Revoke {
            object: Object::Node(doc),
            relation: Relation::Viewer,
            subject: Subject::User(corpus.ada.into()),
        };
        let _ = authz.write(space, Actor::System, &[revoke]).await.unwrap();
        assert!(search.search(&ada, &query).await.unwrap().hits.is_empty());
    }

    #[tokio::test]
    async fn space_grants_reach_every_document_and_links_get_nothing() {
        let corpus = Corpus::default();
        let authz = authz(&corpus);
        let space = Id::from(corpus.eng);
        let _ = authz
            .write(
                space,
                Actor::System,
                &[grant(Object::Space(space), Relation::Viewer, corpus.ada)],
            )
            .await
            .unwrap();

        let ada = Principal::User(corpus.ada);
        assert_eq!(authz.readable_spaces(&ada).await.unwrap(), [corpus.eng]);
        let docs = [corpus.runbook, corpus.budget];
        let readable = authz.filter_readable(&ada, &docs).await.unwrap();
        assert!(readable.contains(&corpus.runbook));
        assert!(!readable.contains(&corpus.budget));

        let link = Principal::ShareLink(ShareId::from_u128(99));
        assert!(authz.readable_spaces(&link).await.unwrap().is_empty());
        assert!(
            authz
                .filter_readable(&link, &docs)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(authz.filter_readable(&ada, &[]).await.unwrap().is_empty());
    }
}
