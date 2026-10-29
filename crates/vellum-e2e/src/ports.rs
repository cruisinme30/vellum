// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! What the service needs from the rest of the server.

use std::fmt;

use async_trait::async_trait;
use vellum_kernel::{Error, SpaceId, UserId};
use vellum_store::BlobPath;

/// What a caller wants to do in a space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Need {
    /// List keys and objects, download ciphertext, enrol or revoke one's
    /// own device.
    View,
    /// Upload and delete ciphertext.
    Edit,
    /// Make a space private (its first key) and revoke other people's
    /// devices.
    Manage,
}

/// Spaces and permissions, as this service sees them. The composition root
/// implements it over the drive's index and the authorization service.
#[async_trait]
pub trait SpaceAccess: Send + Sync + fmt::Debug {
    /// The space's folder in the blob store, or `None` for an unknown
    /// space.
    ///
    /// # Errors
    ///
    /// The index cannot be read.
    async fn root(&self, space: SpaceId) -> Result<Option<BlobPath>, Error>;

    /// Whether `user` holds `need` on the space itself. Deny by default.
    ///
    /// # Errors
    ///
    /// The authorization service cannot be reached.
    async fn allowed(&self, user: UserId, space: SpaceId, need: Need) -> Result<bool, Error>;
}
