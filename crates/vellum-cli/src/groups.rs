// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Group membership on the permission engine (ADR 0057, D273): the
//! identity service names groups, `vellum-authz` keeps who is in them.

use std::collections::BTreeSet;
use std::sync::Arc;

use vellum_authz::{Authz, AuthzStore, Id, Object, Subject, TupleRelation};
use vellum_contracts::{BoxFuture, Service};
use vellum_identity::{GroupMembers, GroupsService, Identity};
use vellum_kernel::{Error, ErrorKind, ObjectId, UserId};

/// The group operations, with members kept by `authz`.
pub fn service<S: AuthzStore + 'static>(
    identity: &Arc<Identity>,
    authz: &Arc<Authz<S>>,
) -> Arc<dyn Service> {
    Arc::new(GroupsService::new(
        Arc::clone(identity),
        Some(Arc::new(AuthzGroups(Arc::clone(authz)))),
    ))
}

/// [`GroupMembers`] over the engine's membership tuples.
pub struct AuthzGroups<S>(pub Arc<Authz<S>>);

impl<S> std::fmt::Debug for AuthzGroups<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthzGroups")
    }
}

fn unavailable(e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Unavailable, format!("group membership: {e}"))
}

const fn user(id: UserId) -> Subject {
    Subject::User(Id(id.as_u128()))
}

impl<S: AuthzStore + 'static> GroupMembers for AuthzGroups<S> {
    fn members(&self, group: ObjectId) -> BoxFuture<'_, Result<BTreeSet<UserId>, Error>> {
        Box::pin(async move {
            let tuples = self.0.store().load_directory().await.map_err(unavailable)?;
            let target = Object::Group(Id(group.as_u128()));
            Ok(tuples
                .into_iter()
                .filter(|t| t.object == target && t.relation == TupleRelation::Member)
                .filter_map(|t| match t.subject {
                    Subject::User(id) => Some(UserId::from_u128(id.0)),
                    _ => None,
                })
                .collect())
        })
    }

    fn change<'a>(
        &'a self,
        group: ObjectId,
        add: &'a [UserId],
        remove: &'a [UserId],
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let add: Vec<Subject> = add.iter().copied().map(user).collect();
            let remove: Vec<Subject> = remove.iter().copied().map(user).collect();
            self.0
                .update_group(Id(group.as_u128()), &add, &remove)
                .await
                .map_err(unavailable)
        })
    }
}

#[cfg(test)]
mod tests {
    use vellum_authz::{MemoryStore, Principal};

    use super::*;

    #[tokio::test]
    async fn members_round_trip_through_the_engine() {
        let authz = Arc::new(Authz::new(MemoryStore::new()));
        let groups = AuthzGroups(Arc::clone(&authz));
        let (group, priya, marco) = (ObjectId::generate(), UserId::generate(), UserId::generate());
        groups.change(group, &[priya, marco], &[]).await.unwrap();
        groups.change(group, &[], &[marco]).await.unwrap();
        assert_eq!(
            groups.members(group).await.unwrap(),
            BTreeSet::from([priya])
        );
        let of_priya = authz
            .groups_of(Principal::User(Id(priya.as_u128())))
            .await
            .unwrap();
        assert!(of_priya.contains(&Id(group.as_u128())));
    }
}
