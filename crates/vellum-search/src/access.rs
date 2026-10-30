// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Ports to the authorization service and per-user state.
//!
//! The index stores no ACLs (D61). Each query is pre-filtered to the spaces
//! the principal can read, then every candidate document passes one exact
//! batch check before anything is returned, so a grant or revocation is
//! visible to search as soon as authorization sees it, with no reindex.
//! Both calls must deny by default: an unknown principal reads nothing.
//! `vellum_authz::Authz` implements [`ReadAccess`] and `vellum_db::Index`
//! implements [`Stars`] for production use.

use std::collections::HashSet;

use async_trait::async_trait;
use vellum_kernel::{DocId, ShareId, SpaceId, UserId};

use crate::error::Result;

/// Who is searching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Principal {
    /// A signed-in person (also a model acting for them through MCP).
    User(UserId),
    /// A visitor holding a share link.
    ShareLink(ShareId),
}

impl Principal {
    /// The person, if the principal is one.
    #[must_use]
    pub const fn user(self) -> Option<UserId> {
        match self {
            Self::User(user) => Some(user),
            Self::ShareLink(_) => None,
        }
    }
}

/// Read permission, answered by the authorization service.
#[async_trait]
pub trait ReadAccess: Send + Sync {
    /// Spaces in which the principal can read at least one document.
    ///
    /// A coarse pre-filter: it may include spaces with only some readable
    /// documents, never spaces with none.
    async fn readable_spaces(
        &self,
        principal: &Principal,
    ) -> Result<Vec<SpaceId>>;

    /// The subset of `docs` the principal can read: the exact check.
    async fn filter_readable(
        &self,
        principal: &Principal,
        docs: &[DocId],
    ) -> Result<HashSet<DocId>>;
}

/// A person's starred documents, for the "starred" filter.
#[async_trait]
pub trait Stars: Send + Sync {
    /// Every document the person has starred.
    async fn starred(&self, user: UserId) -> Result<HashSet<DocId>>;
}
