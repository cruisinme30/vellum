// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! What a workspace administrator does to spaces (ADR 0021, D242).
//!
//! An administrator is usually not a member of the spaces they look
//! after, so these methods skip the drive's own permission checks. The
//! caller decides who may call them: the service serves them only to
//! credentials with the `admin` scope and a recent sign-in.
//!
//! A My Drive belongs to its owner: an administrator sees its size and
//! sets its quota, but cannot add people to it or take it over.

use vellum_kernel::{Actor, SpaceId, UserId};

use super::Drive;
use crate::error::{DriveError, DriveResult};
use crate::model::{Space, SpaceKind, SpaceSettings};
use crate::ports::{Member, SpaceRole};

/// A space as the admin console lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminSpace {
    /// The space.
    pub space: Space,
    /// Bytes its pages, files, trash and versions use.
    pub used: u64,
    /// The people with a role on the whole space.
    pub members: Vec<Member>,
}

impl Drive {
    /// Every space, shared drives first, then by name.
    ///
    /// # Errors
    /// Index or authorization failures.
    #[tracing::instrument(name = "drive.admin_spaces", skip_all, err)]
    pub async fn admin_spaces(&self) -> DriveResult<Vec<AdminSpace>> {
        self.observe("admin_spaces", async {
            let mut spaces = self.index.spaces().await?;
            spaces.sort_by(|a, b| {
                (a.kind != SpaceKind::SharedDrive, &a.name)
                    .cmp(&(b.kind != SpaceKind::SharedDrive, &b.name))
            });
            let mut out = Vec::with_capacity(spaces.len());
            for space in spaces {
                out.push(AdminSpace {
                    used: self.index.usage(space.id).await?,
                    members: self.access.members(space.id).await?,
                    space,
                });
            }
            Ok(out)
        })
        .await
    }

    /// One space with its use and members.
    ///
    /// # Errors
    /// `SpaceNotFound`, or index and authorization failures.
    #[tracing::instrument(
        name = "drive.admin_space",
        skip_all,
        fields(space = %space),
        err
    )]
    pub async fn admin_space(&self, space: SpaceId) -> DriveResult<AdminSpace> {
        self.observe("admin_space", async {
            let space = self.space(space).await?;
            Ok(AdminSpace {
                used: self.index.usage(space.id).await?,
                members: self.access.members(space.id).await?,
                space,
            })
        })
        .await
    }

    /// Creates a shared drive and makes `owner` its owner.
    ///
    /// # Errors
    /// `Invalid` for a bad slug or name, `AlreadyExists` if the slug is
    /// taken, or a store failure.
    #[tracing::instrument(
        name = "drive.admin_create_space",
        skip_all,
        fields(slug = slug),
        err
    )]
    pub async fn admin_create_space(
        &self,
        slug: &str,
        name: &str,
        owner: UserId,
    ) -> DriveResult<AdminSpace> {
        if name.trim().is_empty() {
            return Err(DriveError::Invalid(
                "a shared drive needs a name".into(),
            ));
        }
        let space = self
            .create_space(
                &Actor::System,
                SpaceKind::SharedDrive,
                slug,
                name,
                None,
            )
            .await?;
        self.access
            .set_member(space.id, owner, Some(SpaceRole::Owner))
            .await?;
        self.admin_space(space.id).await
    }

    /// Renames a space and replaces its quota, trash and version settings.
    ///
    /// # Errors
    /// `SpaceNotFound`, `Invalid` for an empty name, or a store failure.
    pub async fn admin_update_space(
        &self,
        space: SpaceId,
        name: Option<&str>,
        settings: SpaceSettings,
    ) -> DriveResult<AdminSpace> {
        if name.is_some_and(|n| n.trim().is_empty()) {
            return Err(DriveError::Invalid("a space needs a name".into()));
        }
        let _ = self
            .update_space(&Actor::System, space, name, settings)
            .await?;
        self.admin_space(space).await
    }

    /// Gives `user` exactly `role` on a shared drive, or removes them.
    /// A shared drive always keeps someone who manages it.
    ///
    /// # Errors
    /// `SpaceNotFound`; `Invalid` for a My Drive, or when the change would
    /// leave the drive with no manager or owner.
    #[tracing::instrument(
        name = "drive.admin_set_member",
        skip_all,
        fields(space = %space),
        err
    )]
    pub async fn admin_set_member(
        &self,
        space: SpaceId,
        user: UserId,
        role: Option<SpaceRole>,
    ) -> DriveResult<AdminSpace> {
        self.observe("admin_set_member", async {
            let found = self.space(space).await?;
            if found.kind == SpaceKind::UserDrive {
                return Err(DriveError::Invalid(
                    "a My Drive belongs to its owner; they share items from it \
                        themselves".into(),
                ));
            }
            let members = self.access.members(space).await?;
            let still_managed =
                members.iter().any(|m| m.user != user && m.role.manages())
                    || role.is_some_and(SpaceRole::manages);
            if !still_managed {
                return Err(DriveError::Invalid(
                    "a shared drive needs at least one manager or owner".into(),
                ));
            }
            self.access.set_member(space, user, role).await
        })
        .await?;
        self.admin_space(space).await
    }
}
