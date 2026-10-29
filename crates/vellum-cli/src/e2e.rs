// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Wires the `e2e` service (ADR 0043) to the drive: a space's folder comes
//! from the drive's index, and access from the drive's permission port, so
//! private spaces follow the same sharing as every other space.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use vellum_drive::{Access, DriveIndex, Permission};
use vellum_e2e::{Need, SpaceAccess};
use vellum_kernel::{Error, SpaceId, UserId};
use vellum_store::BlobPath;

/// [`SpaceAccess`] over the drive's index and permissions.
pub struct DriveSpaces {
    index: Arc<dyn DriveIndex>,
    access: Arc<dyn Access>,
}

impl DriveSpaces {
    /// Adapts the drive's `index` and `access`.
    pub fn new(index: Arc<dyn DriveIndex>, access: Arc<dyn Access>) -> Self {
        Self { index, access }
    }
}

impl fmt::Debug for DriveSpaces {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DriveSpaces").finish_non_exhaustive()
    }
}

#[async_trait]
impl SpaceAccess for DriveSpaces {
    async fn root(&self, space: SpaceId) -> Result<Option<BlobPath>, Error> {
        Ok(self.index.space(space).await?.map(|s| s.root))
    }

    async fn allowed(&self, user: UserId, space: SpaceId, need: Need) -> Result<bool, Error> {
        let permission = match need {
            Need::View => Permission::View,
            Need::Edit => Permission::Edit,
            Need::Manage => Permission::Manage,
        };
        Ok(self
            .access
            .check(user, space, space.into(), permission)
            .await?)
    }
}
