// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Wires the `e2e` service (ADR 0043) to the drive: a space's folder comes
//! from the drive's index, and access from the drive's permission port, so
//! private spaces follow the same sharing as every other space.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use vellum_drive::{Access, DriveIndex, Permission};
use vellum_e2e::{E2e, Need, SpaceAccess};
use vellum_kernel::{Error, SpaceId, UserId};
use vellum_store::{BlobPath, BlobStore};

/// The `e2e` service over the workspace's store, the drive's index and its
/// permissions: device keys and ciphertext for private spaces.
pub fn service(
    store: &Arc<dyn BlobStore>,
    index: &Arc<dyn DriveIndex>,
    access: Arc<dyn Access>,
) -> E2e {
    E2e::new(
        Arc::clone(store),
        Arc::new(DriveSpaces::new(Arc::clone(index), access)),
    )
}

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

    async fn allowed(
        &self,
        user: UserId,
        space: SpaceId,
        need: Need,
    ) -> Result<bool, Error> {
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

/// Puts the private-space guard in front of `store`: private spaces then
/// accept only sealed pages, whichever service writes (D208).
pub fn guarded(
    store: Arc<dyn BlobStore>,
) -> (Arc<dyn BlobStore>, vellum_e2e::PrivateRoots) {
    let roots = vellum_e2e::PrivateRoots::default();
    (
        Arc::new(vellum_e2e::Guard::new(store, roots.clone())),
        roots,
    )
}

/// How often each replica reloads which spaces are private, to pick up
/// key lists changed on other replicas.
const ROOTS_EVERY: std::time::Duration = std::time::Duration::from_secs(15);

/// Loads which spaces are private before serving, then reloads them every
/// [`ROOTS_EVERY`] on this replica.
pub async fn keep_roots_fresh(e2e: Arc<E2e>) {
    match e2e.refresh_roots().await {
        Ok(count) => {
            tracing::info!(private_spaces = count, "private spaces loaded");
        }
        Err(error) => tracing::warn!(%error, "private spaces not loaded yet"),
    }
    drop(tokio::spawn(async move {
        let mut tick = tokio::time::interval(ROOTS_EVERY);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let _ = tick.tick().await;
        loop {
            let _ = tick.tick().await;
            if let Err(error) = e2e.refresh_roots().await {
                tracing::warn!(%error, "private spaces not reloaded");
            }
        }
    }));
}
