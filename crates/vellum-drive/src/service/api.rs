// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Inputs and outputs of the drive operations. Inputs are path parameters
//! plus the query string (GET, DELETE) or JSON body; every type documents
//! itself in the OpenAPI and GraphQL schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_kernel::{Actor, ObjectId, SpaceId, Timestamp, UserId, VersionId};

use crate::model::{
    Activity, ActivityKind, Node, NodeKind, RetentionPolicy, Space, SpaceKind, SpaceSettings,
    VersionRecord,
};

/// No input.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoInput {}

/// Names a space.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SpaceKey {
    /// The space.
    pub space_id: SpaceId,
}

/// Names an item (a folder, page, book, collection or shortcut).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeKey {
    /// The item.
    pub id: ObjectId,
}

/// Names one version of a page.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VersionKey {
    /// The page.
    pub id: ObjectId,
    /// The version.
    pub version_id: VersionId,
}

/// What kind of space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpaceKindView {
    /// A team drive.
    SharedDrive,
    /// One person's My Drive.
    UserDrive,
}

impl From<SpaceKind> for SpaceKindView {
    fn from(kind: SpaceKind) -> Self {
        match kind {
            SpaceKind::SharedDrive => Self::SharedDrive,
            SpaceKind::UserDrive => Self::UserDrive,
        }
    }
}

impl From<SpaceKindView> for SpaceKind {
    fn from(kind: SpaceKindView) -> Self {
        match kind {
            SpaceKindView::SharedDrive => Self::SharedDrive,
            SpaceKindView::UserDrive => Self::UserDrive,
        }
    }
}

/// A space's settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SettingsView {
    /// The most bytes the space may hold, trash and versions included;
    /// no limit when absent.
    pub quota_bytes: Option<u64>,
    /// Days an item stays in the trash before it is purged.
    pub trash_days: u32,
    /// Days every version of a page is kept.
    pub keep_all_days: u32,
    /// Days one version a day is kept after that.
    pub daily_days: u32,
    /// Weeks one version a week is kept after that; forever when absent.
    pub weekly_weeks: Option<u32>,
}

impl From<&SpaceSettings> for SettingsView {
    fn from(s: &SpaceSettings) -> Self {
        Self {
            quota_bytes: s.quota_bytes,
            trash_days: s.trash_days,
            keep_all_days: s.retention.keep_all_days,
            daily_days: s.retention.daily_days,
            weekly_weeks: s.retention.weekly_weeks,
        }
    }
}

impl From<SettingsView> for SpaceSettings {
    fn from(s: SettingsView) -> Self {
        Self {
            quota_bytes: s.quota_bytes,
            trash_days: s.trash_days,
            retention: RetentionPolicy {
                keep_all_days: s.keep_all_days,
                daily_days: s.daily_days,
                weekly_weeks: s.weekly_weeks,
            },
        }
    }
}

/// A space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SpaceView {
    /// The space.
    pub id: SpaceId,
    /// Shared drive or My Drive.
    pub kind: SpaceKindView,
    /// The short name in its folder path.
    pub slug: String,
    /// The display name.
    pub name: String,
    /// The owner of a My Drive.
    pub owner: Option<UserId>,
    /// Quota, trash and version settings.
    pub settings: SettingsView,
}

impl From<&Space> for SpaceView {
    fn from(s: &Space) -> Self {
        Self {
            id: s.id,
            kind: s.kind.into(),
            slug: s.slug.clone(),
            name: s.name.clone(),
            owner: s.owner,
            settings: (&s.settings).into(),
        }
    }
}

/// The caller's My Drive, if they have one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MyDrive {
    /// The space; absent until it is created.
    pub space: Option<SpaceView>,
}

/// Spaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SpaceList {
    /// The spaces.
    pub spaces: Vec<SpaceView>,
}

/// What an item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeKindView {
    /// A folder.
    Folder,
    /// A Markdown page.
    Page,
    /// A book.
    Book,
    /// A collection.
    Collection,
    /// A shortcut to another item.
    Shortcut,
}

impl From<NodeKind> for NodeKindView {
    fn from(kind: NodeKind) -> Self {
        match kind {
            NodeKind::Folder => Self::Folder,
            NodeKind::Page => Self::Page,
            NodeKind::Book => Self::Book,
            NodeKind::Collection => Self::Collection,
            NodeKind::Shortcut => Self::Shortcut,
        }
    }
}

impl From<NodeKindView> for NodeKind {
    fn from(kind: NodeKindView) -> Self {
        match kind {
            NodeKindView::Folder => Self::Folder,
            NodeKindView::Page => Self::Page,
            NodeKindView::Book => Self::Book,
            NodeKindView::Collection => Self::Collection,
            NodeKindView::Shortcut => Self::Shortcut,
        }
    }
}

/// An item in a space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NodeView {
    /// The item.
    pub id: ObjectId,
    /// The space it is in.
    pub space_id: SpaceId,
    /// What it is.
    pub kind: NodeKindView,
    /// The folder it is in; absent at the top of the space.
    pub parent_id: Option<ObjectId>,
    /// Its title.
    pub title: String,
    /// Its path in the workspace folder.
    pub path: String,
    /// Bytes on disk.
    pub size: u64,
    /// When it last changed.
    pub modified: Timestamp,
    /// Who created it, when known.
    pub created_by: Option<UserId>,
    /// What a shortcut points at.
    pub target_id: Option<ObjectId>,
    /// When it was trashed; absent for a live item.
    pub trashed_at: Option<Timestamp>,
}

impl From<&Node> for NodeView {
    fn from(n: &Node) -> Self {
        Self {
            id: n.id,
            space_id: n.space,
            kind: n.kind.into(),
            parent_id: n.parent,
            title: n.title.clone(),
            path: n.path.as_str().to_owned(),
            size: n.size,
            modified: n.modified,
            created_by: n.created_by,
            target_id: n.target,
            trashed_at: n.trash.as_ref().map(|t| t.trashed_at),
        }
    }
}

/// Items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NodeList {
    /// The items.
    pub items: Vec<NodeView>,
}

impl NodeList {
    pub(super) fn of(nodes: &[Node]) -> Self {
        Self {
            items: nodes.iter().map(NodeView::from).collect(),
        }
    }
}

/// The items in a folder, or at the top of a space.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListChildren {
    /// The space.
    pub space_id: SpaceId,
    /// The folder; the top of the space when absent.
    pub parent_id: Option<ObjectId>,
}

/// How many results to return.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Limit {
    /// At most this many, 1 to 200; 50 when absent.
    pub limit: Option<usize>,
}

/// An item's activity.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivityInput {
    /// The item, or a space's ID for the whole space.
    pub id: ObjectId,
    /// At most this many, 1 to 200; 50 when absent.
    pub limit: Option<usize>,
}

/// A recently opened item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RecentItem {
    /// The item.
    pub item: NodeView,
    /// When the caller last opened it.
    pub opened_at: Timestamp,
}

/// The caller's recently opened items, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RecentList {
    /// The items.
    pub items: Vec<RecentItem>,
}

/// Who made a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActorView {
    /// A person.
    User {
        /// The person.
        user: UserId,
    },
    /// A model acting for a person.
    Model {
        /// The person.
        on_behalf_of: UserId,
    },
    /// An edit made outside Vellum.
    External,
    /// Vellum itself.
    System,
}

impl From<Actor> for ActorView {
    fn from(actor: Actor) -> Self {
        match actor {
            Actor::User { user } => Self::User { user },
            Actor::Model { on_behalf_of } => Self::Model { on_behalf_of },
            Actor::External => Self::External,
            _ => Self::System,
        }
    }
}

/// One change in the activity feed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ActivityView {
    /// When it happened.
    pub at: Timestamp,
    /// Who did it.
    pub actor: ActorView,
    /// The item it happened to.
    pub item_id: ObjectId,
    /// What happened: `created`, `edited`, `renamed`, `moved`, `copied`,
    /// `trashed`, `restored`, `purged`, `conflict_copy` or
    /// `version_restored`.
    pub kind: String,
    /// The old name or path, for renames and moves.
    pub from: Option<String>,
    /// The new name or path, for renames and moves.
    pub to: Option<String>,
}

impl From<&Activity> for ActivityView {
    fn from(a: &Activity) -> Self {
        let (kind, from, to) = match &a.kind {
            ActivityKind::Created => ("created", None, None),
            ActivityKind::Edited { .. } => ("edited", None, None),
            ActivityKind::Renamed { from, to } => ("renamed", Some(from.clone()), Some(to.clone())),
            ActivityKind::Moved { from, to } => ("moved", Some(from.clone()), Some(to.clone())),
            ActivityKind::Copied { from } => ("copied", Some(from.to_string()), None),
            ActivityKind::Trashed => ("trashed", None, None),
            ActivityKind::Restored => ("restored", None, None),
            ActivityKind::Purged => ("purged", None, None),
            ActivityKind::ConflictCopy { copy } => ("conflict_copy", None, Some(copy.to_string())),
            ActivityKind::VersionRestored { .. } => ("version_restored", None, None),
        };
        Self {
            at: a.at,
            actor: a.actor.into(),
            item_id: a.node,
            kind: kind.to_owned(),
            from,
            to,
        }
    }
}

/// An activity feed, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ActivityList {
    /// The changes.
    pub events: Vec<ActivityView>,
}

/// How much of its quota a space uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct UsageView {
    /// Bytes used, trash and versions included.
    pub used_bytes: u64,
    /// The quota; no limit when absent.
    pub quota_bytes: Option<u64>,
}

/// Search filters; every filter given must match.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindInput {
    /// Case-insensitive text the title must contain.
    pub title: Option<String>,
    /// Who created the item.
    pub created_by: Option<UserId>,
    /// Modified at or after this time.
    pub modified_after: Option<Timestamp>,
    /// Modified before this time.
    pub modified_before: Option<Timestamp>,
    /// Kinds to keep; all when empty.
    #[serde(default)]
    pub kinds: Vec<NodeKindView>,
    /// A space or folder the item must be in, at any depth.
    pub location: Option<ObjectId>,
    /// Only items the caller starred.
    #[serde(default)]
    pub starred: bool,
    /// At most this many, 1 to 200; 50 when absent.
    pub limit: Option<usize>,
}

/// A page's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ContentView {
    /// The item.
    pub item: NodeView,
    /// The text; absent for a file that is not UTF-8 text.
    pub content: Option<String>,
    /// The content's version tag; send it back as `base_etag` to save.
    pub etag: String,
}

/// A new version of a page's content.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveInput {
    /// The page.
    pub id: ObjectId,
    /// The content the edit started from, as read.
    pub base_content: String,
    /// The etag that came with `base_content`.
    pub base_etag: String,
    /// The edited content.
    pub content: String,
}

/// How a save landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SaveResult {
    /// Nobody else had changed the page; it holds the new content.
    Written,
    /// Changes made meanwhile were merged with the new content.
    Merged,
    /// The changes conflicted: the page is unchanged and the new content
    /// went to a conflict copy.
    Conflicted,
}

/// The outcome of a save.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SaveView {
    /// The page after the save.
    pub item: NodeView,
    /// How the save landed.
    pub result: SaveResult,
    /// The version written; absent after a conflict.
    pub version_id: Option<VersionId>,
    /// The conflict copy holding the new content, after a conflict.
    pub conflict_copy: Option<NodeView>,
}

/// One saved version of a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VersionView {
    /// The version.
    pub id: VersionId,
    /// Its size in bytes.
    pub size: u64,
    /// Who wrote it.
    pub author: ActorView,
}

impl From<&VersionRecord> for VersionView {
    fn from(v: &VersionRecord) -> Self {
        Self {
            id: v.id,
            size: v.size,
            author: v.author.into(),
        }
    }
}

/// A page's versions, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VersionList {
    /// The versions.
    pub versions: Vec<VersionView>,
}

/// One version's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VersionContent {
    /// The version.
    pub id: VersionId,
    /// The text; absent if it is not UTF-8 text.
    pub content: Option<String>,
}

/// A new space.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateSpace {
    /// Shared drive, or the caller's own My Drive.
    pub kind: SpaceKindView,
    /// The short name in its folder path: lowercase letters, digits and
    /// dashes.
    pub slug: String,
    /// The display name.
    pub name: String,
}

/// Changes to a space.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateSpace {
    /// The space.
    pub space_id: SpaceId,
    /// A new display name; unchanged when absent.
    pub name: Option<String>,
    /// The new settings, replacing the old ones.
    pub settings: SettingsView,
}

/// A new folder.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateFolder {
    /// The space.
    pub space_id: SpaceId,
    /// The folder to create it in; the top of the space when absent.
    pub parent_id: Option<ObjectId>,
    /// Its name.
    pub name: String,
}

/// A new page.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreatePage {
    /// The space.
    pub space_id: SpaceId,
    /// The folder to create it in; the top of the space when absent.
    pub parent_id: Option<ObjectId>,
    /// Its title.
    pub title: String,
    /// Its Markdown body.
    #[serde(default)]
    pub body: String,
}

/// A new shortcut.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateShortcut {
    /// The space to put it in.
    pub space_id: SpaceId,
    /// The folder to put it in; the top of the space when absent.
    pub parent_id: Option<ObjectId>,
    /// The item it points at.
    pub target_id: ObjectId,
}

/// A new title.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameInput {
    /// The item.
    pub id: ObjectId,
    /// Its new title.
    pub title: String,
}

/// Where to move or copy an item.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlaceInput {
    /// The item.
    pub id: ObjectId,
    /// The space it goes to.
    pub space_id: SpaceId,
    /// The folder or page it goes into; the top of the space when absent.
    pub parent_id: Option<ObjectId>,
    /// Where it goes among its new siblings, counted without it: 0 is
    /// first. When absent the order is left as it was.
    pub index: Option<usize>,
}

/// A space's tree, or the part of it under one item.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TreeInput {
    /// The space.
    pub space_id: SpaceId,
    /// The folder or page to start under; the whole space when absent.
    pub root_id: Option<ObjectId>,
    /// How many levels to return; all when absent.
    pub depth: Option<usize>,
    /// At most this many items, 1 to 10000; 2000 when absent.
    pub limit: Option<usize>,
}

/// Items in tree order: depth first, siblings in their stored order.
/// Each item names its parent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct TreeView {
    /// The items you can open.
    pub items: Vec<NodeView>,
    /// True when the limit cut the tree short.
    pub truncated: bool,
}

/// Starring or unstarring an item.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StarInput {
    /// The item.
    pub id: ObjectId,
    /// Whether it is starred.
    pub starred: bool,
}

/// A count of what an operation removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Removed {
    /// How many items or versions were removed.
    pub removed: usize,
}

/// An operation that returns nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Done {
    /// Always true.
    pub ok: bool,
}
