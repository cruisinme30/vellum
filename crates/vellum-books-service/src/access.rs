// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The access port: what the caller may do with a book, who they are in
//! `restrict_to` terms, and how section restrictions reach the
//! authorization engine.
//!
//! The gateway has already checked the token's scope. [`AuthzAccess`]
//! answers the rest from the authorization engine, deny by default; tests
//! use their own small adapters.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use tracing::Instrument as _;
use vellum_authz::{Actor, Authz, AuthzStore, Directory, Id, Object, Permission, Relation};
use vellum_books::manifest::Principal as BookPrincipal;
use vellum_contracts::Principal;
use vellum_kernel::{BookId, Error, ErrorKind, ObjectId, Timestamp};
use vellum_store::BlobPath;

use crate::mirror::{self, Restrictions};

/// What a mirror run changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MirrorReport {
    /// Tuples written or removed.
    pub changes: usize,
    /// Sections the engine does not know yet (not indexed).
    pub skipped: usize,
}

/// Decides access to books and keeps the engine's view of section
/// restrictions current.
#[async_trait]
pub trait Access: Send + Sync + 'static {
    /// The caller's relation on the book (the `.book` node), or `None`
    /// when they cannot open it.
    ///
    /// # Errors
    ///
    /// Only when the decision cannot be made.
    async fn relation(
        &self,
        principal: &Principal,
        book: BookId,
        dir: &BlobPath,
    ) -> Result<Option<Relation>, Error>;

    /// The caller as `restrict_to` names people: `user:<id>` and
    /// `group:<id>` for every group they belong to. Empty for share links
    /// and anonymous callers, who then see unrestricted sections only.
    ///
    /// # Errors
    ///
    /// Only when the groups cannot be read.
    async fn principals(&self, principal: &Principal) -> Result<BTreeSet<BookPrincipal>, Error>;

    /// Brings the engine's markers and grants on the book's sections in
    /// line with `restrictions`.
    ///
    /// # Errors
    ///
    /// When the engine cannot be read or written.
    async fn mirror(
        &self,
        book: BookId,
        dir: &BlobPath,
        restrictions: &Restrictions,
    ) -> Result<MirrorReport, Error>;
}

/// Finds the space a book folder belongs to.
pub type SpaceOf = Arc<dyn Fn(&BlobPath) -> Option<Id> + Send + Sync>;

/// [`Access`] backed by the authorization engine.
pub struct AuthzAccess<S> {
    authz: Arc<Authz<S>>,
    space_of: SpaceOf,
}

impl<S> fmt::Debug for AuthzAccess<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthzAccess").finish_non_exhaustive()
    }
}

impl<S> AuthzAccess<S> {
    /// Wraps an engine. `space_of` maps a book folder to its space.
    #[must_use]
    pub fn new(authz: Arc<Authz<S>>, space_of: SpaceOf) -> Self {
        Self { authz, space_of }
    }
}

fn unavailable(e: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::new(ErrorKind::Unavailable, "access could not be checked").with_source(e)
}

/// The engine subject for a request principal, if it has one.
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
    async fn relation(
        &self,
        principal: &Principal,
        book: BookId,
        dir: &BlobPath,
    ) -> Result<Option<Relation>, Error> {
        let (Some(who), Some(space)) = (subject(principal), (self.space_of)(dir)) else {
            return Ok(None);
        };
        let decision = self
            .authz
            .check(space, who, Id(book.as_u128()), Permission::Read)
            .await
            .map_err(unavailable)?;
        Ok(decision.relation.filter(|_| decision.allowed))
    }

    async fn principals(&self, principal: &Principal) -> Result<BTreeSet<BookPrincipal>, Error> {
        let Some(user) = principal.user_id() else {
            return Ok(BTreeSet::new());
        };
        let invalid = |e| Error::new(ErrorKind::Internal, "bad principal").with_source(e);
        let mut out = BTreeSet::from([BookPrincipal::user(user).map_err(invalid)?]);
        let tuples = self
            .authz
            .store()
            .load_directory()
            .await
            .map_err(unavailable)?;
        let now = Timestamp::now().unix_ms();
        let directory = Directory::new(&tuples, now);
        let me = vellum_authz::Principal::User(Id(user.as_u128()));
        let groups: BTreeSet<Id> = tuples
            .iter()
            .filter_map(|t| match t.object {
                Object::Group(g) => Some(g),
                _ => None,
            })
            .collect();
        for group in groups {
            // `expand` gives each member the time their best path through
            // nested groups stops counting.
            if directory
                .expand(group)
                .get(&me)
                .is_some_and(|&until| until > now)
            {
                let _ = out
                    .insert(BookPrincipal::group(ObjectId::from_u128(group.0)).map_err(invalid)?);
            }
        }
        Ok(out)
    }

    async fn mirror(
        &self,
        book: BookId,
        dir: &BlobPath,
        restrictions: &Restrictions,
    ) -> Result<MirrorReport, Error> {
        let Some(space) = (self.space_of)(dir) else {
            return Ok(MirrorReport::default());
        };
        let span = tracing::info_span!(
            "books.mirror",
            book = %book,
            books.changes = tracing::field::Empty,
            books.skipped = tracing::field::Empty,
        );
        async {
            let graph = self
                .authz
                .store()
                .load_space(space)
                .await
                .map_err(unavailable)?;
            let (changes, skipped) = mirror::plan(&graph, restrictions);
            if !changes.is_empty() {
                let _ = self
                    .authz
                    .write(space, Actor::System, &changes)
                    .await
                    .map_err(unavailable)?;
            }
            let span = tracing::Span::current();
            let _ = span.record("books.changes", changes.len());
            let _ = span.record("books.skipped", skipped);
            Ok(MirrorReport {
                changes: changes.len(),
                skipped,
            })
        }
        .instrument(span)
        .await
    }
}
