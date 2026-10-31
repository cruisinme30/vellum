// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Named groups of people (ADR 0057, D273-D275).
//!
//! The permission engine already expands `group:<ID>` grants and keeps who
//! belongs to which group, but nothing created or named a group. Identity
//! keeps each group's name and description with the other identity
//! records; membership stays in the permission engine, reached through the
//! [`GroupMembers`] port, so there is one source of truth for who is in.
//!
//! - [`GroupsService`] serves the admin operations under
//!   `/v1/admin/groups` (admin scope, recent sign-in) and the group search
//!   at `/v1/directory/groups` that the shared people picker uses.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_contracts::{
    BoxFuture, HttpMethod, Operation, RequestContext, Scope, Service,
};
use vellum_kernel::{Error, ErrorKind, ObjectId, Timestamp, UserId};

use crate::identity::{Identity, UserView};
use crate::service::{Directory, DirectoryEntry};
use crate::store::Group;

/// How recent a sign-in group administration needs, as for users.
const FRESH_AUTH: Duration = crate::service::FRESH_AUTH;
/// The longest group name.
const MAX_NAME: usize = 100;
/// The longest description.
const MAX_DESCRIPTION: usize = 500;
/// Search results when the caller does not say.
const DEFAULT_LIMIT: usize = 20;
/// The most search results.
const MAX_LIMIT: usize = 100;

/// Who belongs to which group: the permission engine's membership tuples.
pub trait GroupMembers: Send + Sync + std::fmt::Debug {
    /// The people directly in `group`.
    fn members(
        &self,
        group: ObjectId,
    ) -> BoxFuture<'_, Result<BTreeSet<UserId>, Error>>;
    /// Adds and removes people; spaces granting to the group follow at once.
    fn change<'a>(
        &'a self,
        group: ObjectId,
        add: &'a [UserId],
        remove: &'a [UserId],
    ) -> BoxFuture<'a, Result<(), Error>>;
}

/// A group as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GroupView {
    /// The group's ID; grants name it as `group:<ID>`.
    pub id: ObjectId,
    /// The display name, unique ignoring case.
    pub name: String,
    /// What the group is for.
    pub description: String,
    /// When it was created.
    pub created_at: Timestamp,
}

impl From<&Group> for GroupView {
    fn from(g: &Group) -> Self {
        Self {
            id: g.id,
            name: g.name.clone(),
            description: g.description.clone(),
            created_at: g.created_at,
        }
    }
}

fn check_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME
        || name.chars().any(char::is_control)
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("a group name has 1 to {MAX_NAME} characters"),
        ));
    }
    Ok(name.to_owned())
}

fn check_description(text: &str) -> Result<String, Error> {
    let text = text.trim();
    if text.chars().count() > MAX_DESCRIPTION {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "a group description has at most {MAX_DESCRIPTION} characters"
            ),
        ));
    }
    Ok(text.to_owned())
}

fn not_found() -> Error {
    Error::new(ErrorKind::NotFound, "group not found")
}

fn name_taken(
    groups: &std::collections::BTreeMap<String, Group>,
    name: &str,
    except: ObjectId,
) -> bool {
    let wanted = name.to_lowercase();
    groups
        .values()
        .any(|g| g.id != except && g.name.to_lowercase() == wanted)
}

/// The group records.
impl Identity {
    /// Every group, by name.
    #[must_use]
    pub fn groups(&self) -> Vec<GroupView> {
        let mut groups: Vec<GroupView> = self
            .store()
            .read(|s| s.groups.values().map(GroupView::from).collect());
        groups.sort_by_key(|g| g.name.to_lowercase());
        groups
    }

    /// One group.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown group.
    pub fn group(&self, id: ObjectId) -> Result<GroupView, Error> {
        self.store()
            .read(|s| s.groups.get(&id.to_string()).map(GroupView::from))
            .ok_or_else(not_found)
    }

    /// Groups whose name contains `query`, ignoring case, by name.
    #[must_use]
    pub fn search_groups(&self, query: &str, limit: usize) -> Vec<GroupView> {
        let query = query.trim().to_lowercase();
        let mut found: Vec<GroupView> = self
            .groups()
            .into_iter()
            .filter(|g| g.name.to_lowercase().contains(&query))
            .collect();
        // Names that start with the query first.
        found.sort_by_key(|g| !g.name.to_lowercase().starts_with(&query));
        found.truncate(limit);
        found
    }

    /// Creates a group.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] for a bad name or description, and
    /// [`ErrorKind::Conflict`] when another group has the name.
    pub fn create_group(
        &self,
        name: &str,
        description: &str,
    ) -> Result<GroupView, Error> {
        let group = Group {
            id: ObjectId::generate(),
            name: check_name(name)?,
            description: check_description(description)?,
            created_at: Timestamp::now(),
            external_id: None,
        };
        self.store().write(|s| {
            if name_taken(&s.groups, &group.name, group.id) {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "another group has this name",
                ));
            }
            let view = GroupView::from(&group);
            let _ = s.groups.insert(group.id.to_string(), group);
            Ok(view)
        })?
    }

    /// Renames a group or changes its description.
    ///
    /// # Errors
    ///
    /// As [`Identity::create_group`], and [`ErrorKind::NotFound`].
    pub fn update_group(
        &self,
        id: ObjectId,
        name: Option<&str>,
        description: Option<&str>,
    ) -> Result<GroupView, Error> {
        let name = name.map(check_name).transpose()?;
        let description = description.map(check_description).transpose()?;
        self.store().write(|s| {
            if let Some(name) = &name
                && name_taken(&s.groups, name, id)
            {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "another group has this name",
                ));
            }
            let group =
                s.groups.get_mut(&id.to_string()).ok_or_else(not_found)?;
            if let Some(name) = name {
                group.name = name;
            }
            if let Some(description) = description {
                group.description = description;
            }
            Ok(GroupView::from(&*group))
        })?
    }

    pub(crate) fn forget_group(&self, id: ObjectId) -> Result<(), Error> {
        self.store().write(|s| {
            s.groups
                .remove(&id.to_string())
                .map(|_| ())
                .ok_or_else(not_found)
        })?
    }
}

/// Serves the group operations.
#[derive(Debug, Clone)]
pub struct GroupsService {
    identity: Arc<Identity>,
    members: Option<Arc<dyn GroupMembers>>,
}

/// A group named in the path.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupKey {
    /// The group.
    pub id: ObjectId,
}

/// A new group.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateGroup {
    /// The display name, unique ignoring case.
    pub name: String,
    /// What the group is for.
    #[serde(default)]
    pub description: String,
}

/// Changes to a group.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateGroup {
    /// The group.
    pub id: ObjectId,
    /// A new name.
    pub name: Option<String>,
    /// A new description.
    pub description: Option<String>,
}

/// People to add to and remove from a group.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeMembers {
    /// The group.
    pub id: ObjectId,
    /// People to add.
    #[serde(default)]
    pub add: Vec<UserId>,
    /// People to remove.
    #[serde(default)]
    pub remove: Vec<UserId>,
}

/// Every group.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GroupList {
    /// The groups, by name.
    pub groups: Vec<GroupView>,
}

/// A group and the people in it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GroupMembersView {
    /// The group.
    pub group: GroupView,
    /// Its members, by name.
    pub members: Vec<UserView>,
}

/// A deleted group.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GroupDeleted {
    /// The group that is gone.
    pub id: ObjectId,
}

/// A group search.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupSearch {
    /// Text the name contains, ignoring case; empty lists every group.
    #[serde(default)]
    pub q: String,
    /// The most results, 1 to 100; 20 when absent.
    pub limit: Option<usize>,
}

impl GroupsService {
    /// Serves `identity`'s groups. Without `members` (no permission
    /// engine, as when only printing the API) membership calls answer
    /// "unavailable".
    #[must_use]
    pub fn new(
        identity: Arc<Identity>,
        members: Option<Arc<dyn GroupMembers>>,
    ) -> Self {
        Self { identity, members }
    }

    fn port(&self) -> Result<&Arc<dyn GroupMembers>, Error> {
        self.members.as_ref().ok_or_else(|| {
            Error::new(
                ErrorKind::Unavailable,
                "group membership needs the workspace index",
            )
        })
    }

    async fn members_view(
        &self,
        id: ObjectId,
    ) -> Result<GroupMembersView, Error> {
        let group = self.identity.group(id)?;
        let ids = self.port()?.members(id).await?;
        let mut members: Vec<UserView> = ids
            .into_iter()
            .filter_map(|user| self.identity.user(user).ok())
            .collect();
        members.sort_by_key(|u| u.name.to_lowercase());
        Ok(GroupMembersView { group, members })
    }

    async fn change_members(
        &self,
        input: ChangeMembers,
    ) -> Result<GroupMembersView, Error> {
        let _ = self.identity.group(input.id)?;
        for user in &input.add {
            let found = self.identity.user(*user)?;
            if found.disabled {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "{} is blocked and cannot join a group",
                        found.name
                    ),
                ));
            }
        }
        self.port()?
            .change(input.id, &input.add, &input.remove)
            .await?;
        self.members_view(input.id).await
    }

    async fn delete(&self, id: ObjectId) -> Result<GroupDeleted, Error> {
        let _ = self.identity.group(id)?;
        // Members go first, so access through the group ends even if the
        // record could not be removed; a retry finishes the job.
        if let Some(port) = &self.members {
            let everyone: Vec<UserId> =
                port.members(id).await?.into_iter().collect();
            if !everyone.is_empty() {
                port.change(id, &[], &everyone).await?;
            }
        }
        self.identity.forget_group(id)?;
        Ok(GroupDeleted { id })
    }

    fn search(&self, input: &GroupSearch) -> Directory {
        let limit = input.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        Directory {
            entries: self
                .identity
                .search_groups(&input.q, limit)
                .into_iter()
                .map(|g| DirectoryEntry {
                    kind: "group".to_owned(),
                    principal: format!("group:{}", g.id),
                    detail: if g.description.is_empty() {
                        "Group".to_owned()
                    } else {
                        g.description
                    },
                    name: g.name,
                })
                .collect(),
        }
    }
}

/// A handler that gets its own handle on the service.
fn with<I, O, F, Fut>(
    service: &Arc<GroupsService>,
    f: F,
) -> impl Fn(RequestContext, I) -> Fut + Send + Sync + 'static
where
    F: Fn(Arc<GroupsService>, RequestContext, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, Error>> + Send + 'static,
{
    let service = Arc::clone(service);
    move |ctx, input| f(Arc::clone(&service), ctx, input)
}

impl Service for GroupsService {
    fn name(&self) -> &'static str {
        "groups"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let admin = |op: Operation| op.fresh_auth(FRESH_AUTH);
        vec![
            admin(Operation::query(
                "groups.list",
                "/v1/admin/groups",
                Scope::Admin,
                with(&self, |s, _ctx, _: crate::service::Empty| async move {
                    Ok::<_, Error>(GroupList {
                        groups: s.identity.groups(),
                    })
                }),
            ))
            .summary("List every group"),
            admin(Operation::mutation(
                "groups.create",
                HttpMethod::Post,
                "/v1/admin/groups",
                Scope::Admin,
                with(&self, |s, _ctx, input: CreateGroup| async move {
                    s.identity.create_group(&input.name, &input.description)
                }),
            ))
            .summary("Create a group"),
            admin(Operation::mutation(
                "groups.update",
                HttpMethod::Patch,
                "/v1/admin/groups/{id}",
                Scope::Admin,
                with(&self, |s, _ctx, input: UpdateGroup| async move {
                    s.identity.update_group(
                        input.id,
                        input.name.as_deref(),
                        input.description.as_deref(),
                    )
                }),
            ))
            .summary("Rename a group or change its description"),
            admin(Operation::mutation(
                "groups.delete",
                HttpMethod::Delete,
                "/v1/admin/groups/{id}",
                Scope::Admin,
                with(&self, |s, _ctx, input: GroupKey| async move {
                    s.delete(input.id).await
                }),
            ))
            .summary("Delete a group; its members lose what it granted"),
            admin(Operation::query(
                "groups.members",
                "/v1/admin/groups/{id}/members",
                Scope::Admin,
                with(&self, |s, _ctx, input: GroupKey| async move {
                    s.members_view(input.id).await
                }),
            ))
            .summary("List a group's members"),
            admin(Operation::mutation(
                "groups.change_members",
                HttpMethod::Post,
                "/v1/admin/groups/{id}/members",
                Scope::Admin,
                with(&self, |s, _ctx, input: ChangeMembers| async move {
                    s.change_members(input).await
                }),
            ))
            .summary("Add people to a group and remove others"),
            Operation::query(
                "groups.search",
                "/v1/directory/groups",
                Scope::Read,
                with(
                    &self,
                    |s, ctx: RequestContext, input: GroupSearch| async move {
                        let _ = ctx.principal.require_user()?;
                        Ok::<_, Error>(s.search(&input))
                    },
                ),
            )
            .summary("Find groups by name, for choosing who to share with"),
        ]
    }
}
