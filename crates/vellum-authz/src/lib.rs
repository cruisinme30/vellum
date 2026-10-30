// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Deny-by-default relationship-based authorization over the index.
//!
//! Vellum's authorization is Zanzibar-shaped (ADR 0008):
//!
//! 1. **Tuples** (`grants`): `object#relation@subject`, where the object is
//!    a space, a node (folder, document, book, section) or a group, and the
//!    subject is a user, a share link or a group's members.
//! 2. **Ancestry** (`page_ancestors`): the folder tree, maintained by the
//!    index from the files on disk.
//! 3. **Effective access** (`effective_access`): each principal's strongest
//!    relation on each object, materialised from the two above on every
//!    write and stamped with the space's **authz epoch**.
//!
//! A check is one indexed read of (3) and denies when no row exists. A
//! write changes (1), recomputes the affected rows of (3) and bumps the
//! epoch in one transaction, so changes are visible at once.
//!
//! ```
//! # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
//! use std::collections::BTreeMap;
//! use vellum_authz::{Actor, Authz, Change, Id, MemoryStore, Object, Permission, Principal, Relation, Subject, Tuple};
//!
//! let (space, folder, doc, priya) = (Id(1), Id(2), Id(3), Id(4));
//! let store = MemoryStore::new();
//! store.set_tree(space, BTreeMap::from([(folder, None), (doc, Some(folder))]))?;
//! let authz = Authz::new(store);
//!
//! let reader = Principal::User(priya);
//! assert!(!authz.check(space, reader, doc, Permission::Read).await?.allowed);
//!
//! let grant = Tuple::role(Object::Node(folder), Relation::Viewer, Subject::User(priya));
//! authz.write(space, Actor::System, &[Change::Grant(grant)]).await?;
//! assert!(authz.check(space, reader, doc, Permission::Read).await?.allowed);
//! assert!(!authz.check(space, reader, doc, Permission::Edit).await?.allowed);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! # }).unwrap();
//! ```

mod engine;
mod graph;
mod memory;
mod model;
mod sharded;
mod sql;
mod store;

pub use engine::{Actor, Authz, Change, Clock, Decision, Error};
pub use graph::{
    Access, Directory, FOREVER, GraphError, Levels, Materialised, SpaceGraph,
    materialise, next_expiry,
};
pub use memory::MemoryStore;
pub use model::{
    Id, Object, Permission, Principal, Relation, Subject, Tuple, TupleRelation,
};
pub use sharded::ShardedStore;
pub use sql::SqlStore;
pub use store::{
    AuthzStore, Commit, EffectiveRow, Scope, StoreError, TupleKey,
};
