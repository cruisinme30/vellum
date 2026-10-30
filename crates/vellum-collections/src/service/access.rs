// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The access port: may this caller read or edit this collection?
//!
//! The gateway has already checked the token's scope. This port answers
//! the per-object question, deny by default. [`AuthzAccess`] answers it
//! from the authorization index; tests use their own small adapters.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use vellum_authz::{Authz, AuthzStore, Id, Permission};
use vellum_contracts::Principal;
use vellum_kernel::{CollectionId, Error, ErrorKind};
use vellum_store::BlobPath;

/// What the caller wants to do with a collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Need {
    /// Read the schema, rows and computed values.
    Read,
    /// Create, change or delete rows.
    Edit,
}

impl Need {
    /// The name used in errors and spans.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Edit => "edit",
        }
    }
}

/// Decides per-object access to collections.
#[async_trait]
pub trait Access: Send + Sync + 'static {
    /// Whether `principal` may do `need` on the collection with this ID,
    /// stored in the folder at `path`.
    ///
    /// # Errors
    ///
    /// Only when the decision cannot be made (the index is down). A
    /// denial is `Ok(false)`.
    async fn allowed(
        &self,
        principal: &Principal,
        collection: CollectionId,
        path: &BlobPath,
        need: Need,
    ) -> Result<bool, Error>;

    /// Whether `principal` may create a collection inside `folder`
    /// (`need` is [`Need::Edit`]). Denied unless an adapter says otherwise.
    ///
    /// # Errors
    ///
    /// Only when the decision cannot be made.
    async fn allowed_in(
        &self,
        principal: &Principal,
        folder: &BlobPath,
        need: Need,
    ) -> Result<bool, Error> {
        let _ = (principal, folder, need);
        Ok(false)
    }
}

/// Finds the space a collection folder belongs to.
pub type SpaceOf = Arc<dyn Fn(&BlobPath) -> Option<Id> + Send + Sync>;

/// Finds the authz node of a drive folder, from the index; `None` when
/// the path is not a live folder of a known space (D305).
pub type NodeOf =
    Arc<dyn Fn(BlobPath) -> futures::future::BoxFuture<'static, Option<Id>> + Send + Sync>;

/// [`Access`] backed by the authorization engine.
///
/// The collection folder is the authz node, identified by the collection
/// ID from `schema.yaml`; the index registers it in the folder tree like
/// any other node. A collection in no known space is denied.
pub struct AuthzAccess<S> {
    authz: Arc<Authz<S>>,
    space_of: SpaceOf,
    node_of: Option<NodeOf>,
}

impl<S> fmt::Debug for AuthzAccess<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthzAccess").finish_non_exhaustive()
    }
}

impl<S> AuthzAccess<S> {
    /// Wraps an authorization engine. `space_of` maps a folder path to its
    /// space.
    #[must_use]
    pub fn new(authz: Arc<Authz<S>>, space_of: SpaceOf) -> Self {
        Self {
            authz,
            space_of,
            node_of: None,
        }
    }

    /// Lets the adapter answer [`Access::allowed_in`] by looking up a
    /// folder's authz node; without it, creating collections is denied.
    #[must_use]
    pub fn with_folders(mut self, node_of: NodeOf) -> Self {
        self.node_of = Some(node_of);
        self
    }

    async fn check(
        &self,
        principal: &Principal,
        space: Option<Id>,
        node: Id,
        need: Need,
    ) -> Result<bool, Error>
    where
        S: AuthzStore,
    {
        let (Some(who), Some(space)) = (subject(principal), space) else {
            return Ok(false);
        };
        let permission = match need {
            Need::Read => Permission::Read,
            Need::Edit => Permission::Edit,
        };
        let decision = self
            .authz
            .check(space, who, node, permission)
            .await
            .map_err(|e| {
                Error::new(ErrorKind::Unavailable, "access could not be checked").with_source(e)
            })?;
        Ok(decision.allowed)
    }
}

/// The authz subject for a request principal, if it has one.
fn subject(principal: &Principal) -> Option<vellum_authz::Principal> {
    principal
        .user_id()
        .map(|u| vellum_authz::Principal::User(Id(u.as_u128())))
        .or_else(|| {
            principal
                .share_id()
                .map(|s| vellum_authz::Principal::ShareLink(Id(s.as_u128())))
        })
}

#[async_trait]
impl<S: AuthzStore + 'static> Access for AuthzAccess<S> {
    async fn allowed(
        &self,
        principal: &Principal,
        collection: CollectionId,
        path: &BlobPath,
        need: Need,
    ) -> Result<bool, Error> {
        self.check(
            principal,
            (self.space_of)(path),
            Id(collection.as_u128()),
            need,
        )
        .await
    }

    async fn allowed_in(
        &self,
        principal: &Principal,
        folder: &BlobPath,
        need: Need,
    ) -> Result<bool, Error> {
        let Some(node_of) = &self.node_of else {
            return Ok(false);
        };
        let Some(node) = node_of(folder.clone()).await else {
            return Ok(false);
        };
        self.check(principal, (self.space_of)(folder), node, need)
            .await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use vellum_authz::{Actor, Change, MemoryStore, Object, Relation, Subject, Tuple};
    use vellum_contracts::{CredentialKind, Scope, ScopeSet};
    use vellum_kernel::{ShareId, Timestamp, UserId};

    use super::*;

    #[tokio::test]
    async fn checks_the_collection_node() {
        let space = Id(1);
        let collection = CollectionId::generate();
        let node = Id(collection.as_u128());
        let store = MemoryStore::new();
        store
            .set_tree(space, BTreeMap::from([(node, None), (Id(77), None)]))
            .unwrap();
        let authz = Arc::new(Authz::new(store));
        let user = UserId::generate();
        let grant = Tuple::role(
            Object::Node(node),
            Relation::Viewer,
            Subject::User(Id(user.as_u128())),
        );
        let folder_grant = Tuple::role(
            Object::Node(Id(77)),
            Relation::Editor,
            Subject::User(Id(user.as_u128())),
        );
        let _ = authz
            .write(
                space,
                Actor::System,
                &[Change::Grant(grant), Change::Grant(folder_grant)],
            )
            .await
            .unwrap();
        let access = AuthzAccess::new(
            authz,
            Arc::new(|p: &BlobPath| p.as_str().starts_with("drives/").then_some(Id(1))),
        );
        let ana = Principal::user(
            user,
            CredentialKind::Session,
            ScopeSet::from_scopes([Scope::WriteDirect]),
            Timestamp::now(),
        );
        let path = BlobPath::new("drives/shared/Ops/Vendors.collection").unwrap();
        let elsewhere = BlobPath::new("tasks/ana/Vendors.collection").unwrap();
        assert!(
            access
                .allowed(&ana, collection, &path, Need::Read)
                .await
                .unwrap()
        );
        assert!(
            !access
                .allowed(&ana, collection, &path, Need::Edit)
                .await
                .unwrap()
        );
        assert!(
            !access
                .allowed(&ana, collection, &elsewhere, Need::Read)
                .await
                .unwrap()
        );
        let anon = Principal::anonymous();
        assert!(
            !access
                .allowed(&anon, collection, &path, Need::Read)
                .await
                .unwrap()
        );
        let link = Principal::share_link(ShareId::generate());
        assert!(
            !access
                .allowed(&link, collection, &path, Need::Read)
                .await
                .unwrap()
        );
        let ops = BlobPath::new("drives/shared/Ops").unwrap();
        assert!(!access.allowed_in(&ana, &ops, Need::Edit).await.unwrap());
        let folder = Id(77);
        let access = access.with_folders(Arc::new(
            move |_: BlobPath| -> futures::future::BoxFuture<'static, Option<Id>> {
                Box::pin(async move { Some(folder) })
            },
        ));
        assert!(access.allowed_in(&ana, &ops, Need::Edit).await.unwrap());
        assert!(!access.allowed_in(&anon, &ops, Need::Edit).await.unwrap());
        assert_eq!(Need::Edit.as_str(), "edit");
        assert!(format!("{access:?}").contains("AuthzAccess"));
    }
}
