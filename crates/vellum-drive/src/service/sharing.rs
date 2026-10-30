// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The sharing operations (#95): who has access to an item, grants to
//! people and groups, and share links.

use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_contracts::{HttpMethod, Operation, RequestContext, Scope};
use vellum_kernel::{Error, ErrorKind, ObjectId, ShareId, Timestamp, UserId};

use super::api::{Done, NodeKey, NodeView};
use super::{DriveService, Handles, actor};
use crate::model::NodeKind;
use crate::sharing::{Grant, Grantee, LinkInfo, Role, Sharing, SharingState};

/// A role granted on an item.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RoleView {
    /// Read.
    Viewer,
    /// Read and comment.
    Commenter,
    /// Read, comment and edit.
    Editor,
    /// Edit, plus share, move and trash.
    Manager,
    /// Everything; listed, never granted through `drive.share`.
    Owner,
}

impl From<Role> for RoleView {
    fn from(role: Role) -> Self {
        match role {
            Role::Viewer => Self::Viewer,
            Role::Commenter => Self::Commenter,
            Role::Editor => Self::Editor,
            Role::Manager => Self::Manager,
            Role::Owner => Self::Owner,
        }
    }
}

impl From<RoleView> for Role {
    fn from(role: RoleView) -> Self {
        match role {
            RoleView::Viewer => Self::Viewer,
            RoleView::Commenter => Self::Commenter,
            RoleView::Editor => Self::Editor,
            RoleView::Manager => Self::Manager,
            RoleView::Owner => Self::Owner,
        }
    }
}

/// A grant made on the item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GrantView {
    /// `user:<id>` or `group:<id>`.
    pub principal: String,
    /// The person's handle, when known.
    pub handle: Option<String>,
    /// The role.
    pub role: RoleView,
    /// When it stops counting; unset for no end.
    pub expires_at: Option<Timestamp>,
}

/// A share link, without its token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct LinkView {
    /// The link.
    pub id: ShareId,
    /// Whether it asks for a password.
    pub has_password: bool,
    /// When it stops working; unset for no end.
    pub expires_at: Option<Timestamp>,
    /// Who made it.
    pub created_by: UserId,
    /// When.
    pub created_at: Timestamp,
    /// Whether it still opens (not revoked, not expired).
    pub live: bool,
}

impl From<&LinkInfo> for LinkView {
    fn from(info: &LinkInfo) -> Self {
        Self {
            id: info.id,
            has_password: info.has_password,
            expires_at: info.expires_at,
            created_by: info.created_by,
            created_at: info.created_at,
            live: info.live,
        }
    }
}

/// Who has access to an item through grants on it, and its share links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SharingView {
    /// The item.
    pub item: NodeView,
    /// Grants on this item, strongest role first. Access inherited from a
    /// folder or the space is not listed.
    pub grants: Vec<GrantView>,
    /// Share links, newest first, revoked ones included.
    pub links: Vec<LinkView>,
    /// Whether the item can have share links (pages only, with a public
    /// origin configured).
    pub links_allowed: bool,
}

/// A grant to make.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShareInput {
    /// The item.
    pub id: ObjectId,
    /// `user:<id>` or `group:<id>`.
    pub principal: String,
    /// The role; `owner` is refused.
    pub role: RoleView,
    /// When it stops counting; leave unset for no end.
    #[serde(default)]
    pub expires_at: Option<Timestamp>,
}

/// A grant to remove.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnshareInput {
    /// The item.
    pub id: ObjectId,
    /// `user:<id>` or `group:<id>`.
    pub principal: String,
}

/// A share link to make.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateLinkInput {
    /// The page.
    pub id: ObjectId,
    /// When it stops working; leave unset for no end.
    #[serde(default)]
    pub expires_at: Option<Timestamp>,
    /// A password the visitor must give, 8 characters or more.
    #[serde(default)]
    pub password: Option<String>,
}

/// A new share link, with the only copy of its token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CreatedLink {
    /// The link.
    pub link: LinkView,
    /// The token. It is not stored and cannot be shown again.
    pub token: String,
    /// The link to hand out, on the public origin.
    pub url: String,
}

/// Names a share link.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkKey {
    /// The link.
    pub link_id: ShareId,
}

fn view(
    state: &SharingState,
    links_allowed: bool,
    handles: Option<&Handles>,
) -> SharingView {
    SharingView {
        item: NodeView::from(&state.node),
        grants: state
            .grants
            .iter()
            .map(|g| GrantView {
                principal: g.grantee.to_string(),
                handle: match (g.grantee, handles) {
                    (Grantee::User(user), Some(handles)) => handles(user),
                    _ => None,
                },
                role: g.role.into(),
                expires_at: g.expires_at,
            })
            .collect(),
        links: state.links.iter().map(LinkView::from).collect(),
        links_allowed: links_allowed && state.node.kind == NodeKind::Page,
    }
}

fn unavailable() -> Error {
    Error::new(
        ErrorKind::Unavailable,
        "sharing is not set up on this server",
    )
}

impl DriveService {
    fn sharing(&self) -> Result<&Arc<Sharing>, Error> {
        self.sharing.as_ref().ok_or_else(unavailable)
    }

    async fn sharing_view(
        &self,
        ctx: &RequestContext,
        id: ObjectId,
    ) -> Result<SharingView, Error> {
        let sharing = self.sharing()?;
        let state = sharing.state(&actor(ctx)?, id).await?;
        Ok(view(
            &state,
            sharing.public_origin().is_some(),
            self.handles.as_ref(),
        ))
    }

    async fn sharing_state(
        &self,
        ctx: &RequestContext,
        input: NodeKey,
    ) -> Result<SharingView, Error> {
        self.sharing_view(ctx, input.id).await
    }

    async fn share(
        &self,
        ctx: &RequestContext,
        input: ShareInput,
    ) -> Result<SharingView, Error> {
        let grant = Grant {
            grantee: Grantee::parse(&input.principal)?,
            role: input.role.into(),
            expires_at: input.expires_at,
        };
        self.sharing()?
            .share(&actor(ctx)?, input.id, &grant)
            .await?;
        self.sharing_view(ctx, input.id).await
    }

    async fn unshare(
        &self,
        ctx: &RequestContext,
        input: UnshareInput,
    ) -> Result<SharingView, Error> {
        let grantee = Grantee::parse(&input.principal)?;
        self.sharing()?
            .unshare(&actor(ctx)?, input.id, grantee)
            .await?;
        self.sharing_view(ctx, input.id).await
    }

    async fn create_link(
        &self,
        ctx: &RequestContext,
        input: CreateLinkInput,
    ) -> Result<CreatedLink, Error> {
        let sharing = Arc::clone(self.sharing()?);
        let origin = sharing.public_origin().map(str::to_owned).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                "share links need a public origin (VELLUM_PUBLIC_PAGES_URL)",
            )
        })?;
        let created = sharing
            .create_link(
                &actor(ctx)?,
                input.id,
                input.expires_at,
                input.password.as_deref(),
            )
            .await?;
        let token = created.token.expose().to_owned();
        Ok(CreatedLink {
            link: LinkView::from(&created.info),
            url: format!("{origin}/s/{token}"),
            token,
        })
    }

    async fn revoke_link(
        &self,
        ctx: &RequestContext,
        input: LinkKey,
    ) -> Result<Done, Error> {
        self.sharing()?
            .revoke_link(&actor(ctx)?, input.link_id)
            .await?;
        Ok(Done { ok: true })
    }

    pub(super) fn sharing_ops(self: &Arc<Self>) -> Vec<Operation> {
        vec![
            op!(self, query "drive.sharing", "/v1/drive/items/{id}/sharing",
                sharing_state,
                "List who has access to an item and its share links"),
            op!(self, Put "drive.share", "/v1/drive/items/{id}/grants", share,
                "Give a person or group a role on an item"),
            op!(self, Delete "drive.unshare", "/v1/drive/items/{id}/grants/{pri\
                ncipal}", unshare,
                "Remove the roles a person or group holds on an item"),
            op!(self, Post "drive.create_link", "/v1/drive/items/{id}/links",
                create_link,
                "Make a share link to a page, optionally expiring and \
                    password-protected"),
            op!(self, Delete "drive.revoke_link", "/v1/drive/links/{link_id}",
                revoke_link,
                "Revoke a share link"),
        ]
    }
}
