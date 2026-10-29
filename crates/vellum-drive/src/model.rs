// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The drive's domain types.

use serde::{Deserialize, Serialize};
use vellum_kernel::{Actor, EventId, ObjectId, SpaceId, Timestamp, UserId, VersionId};
use vellum_store::BlobPath;

/// What a space is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceKind {
    /// A shared drive, owned by a team.
    SharedDrive,
    /// A person's My Drive.
    UserDrive,
}

/// Per-space settings, kept in the space's `.vellum-folder.yaml` so a
/// rebuilt index loses nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpaceSettings {
    /// Storage quota in bytes, counting content, trash and versions.
    /// `None` is unlimited.
    pub quota_bytes: Option<u64>,
    /// Days an item stays in the trash before it is purged.
    pub trash_days: u32,
    /// How long versions are kept.
    pub retention: RetentionPolicy,
}

impl Default for SpaceSettings {
    fn default() -> Self {
        Self {
            quota_bytes: None,
            trash_days: 30,
            retention: RetentionPolicy::default(),
        }
    }
}

/// Version retention (R-DR-10).
///
/// Keep every version for `keep_all_days`, then the newest version of
/// each day for `daily_days` more, then the
/// newest of each ISO week for `weekly_weeks` more (`None`: forever).
/// The newest version of a document is always kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RetentionPolicy {
    /// Days during which every version is kept.
    pub keep_all_days: u32,
    /// Days, after that, during which one version per day is kept.
    pub daily_days: u32,
    /// Weeks, after that, during which one version per week is kept.
    pub weekly_weeks: Option<u32>,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            keep_all_days: 30,
            daily_days: 90,
            weekly_weeks: None,
        }
    }
}

/// A space: a shared drive or a My Drive. The shard key everywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    /// Identity, also the ID in the root `.vellum-folder.yaml`.
    pub id: SpaceId,
    /// Shared drive or My Drive.
    pub kind: SpaceKind,
    /// URL- and path-safe name.
    pub slug: String,
    /// Display name.
    pub name: String,
    /// The person whose My Drive this is.
    pub owner: Option<UserId>,
    /// Quota, trash and retention settings.
    pub settings: SpaceSettings,
    /// The space's directory.
    pub root: BlobPath,
}

impl Space {
    /// The space as a node ID, for permission checks on the space itself.
    #[must_use]
    pub fn node_id(&self) -> ObjectId {
        self.id.into()
    }
}

/// The kind of a drive node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    /// A folder.
    Folder,
    /// A Markdown page.
    Page,
    /// A book (`name.book/`), moved and copied as a unit.
    Book,
    /// A collection (`name.collection/`), moved and copied as a unit.
    Collection,
    /// A shortcut to another node; never grants access.
    Shortcut,
}

impl NodeKind {
    /// The suffix the node's file or directory name carries.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Folder => "",
            Self::Page => ".md",
            Self::Book => ".book",
            Self::Collection => ".collection",
            Self::Shortcut => ".shortcut.yaml",
        }
    }

    /// True for nodes stored as a directory.
    #[must_use]
    pub const fn is_dir(self) -> bool {
        matches!(self, Self::Folder | Self::Book | Self::Collection)
    }

    /// True for nodes that hold other drive nodes.
    #[must_use]
    pub const fn is_container(self) -> bool {
        matches!(self, Self::Folder)
    }
}

/// How a node came to be in the trash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashState {
    /// When it was trashed.
    pub trashed_at: Timestamp,
    /// Who trashed it.
    pub trashed_by: Actor,
    /// True when the node itself was trashed; false when it went with a
    /// trashed folder. Restoring a folder brings back only the nodes that
    /// went with it, so restore is exact (R-DR-4).
    pub explicitly_trashed: bool,
    /// The explicitly trashed node whose entry holds this one.
    pub entry: ObjectId,
    /// Where the node lived before it was trashed.
    #[serde(with = "crate::files::blob_path")]
    pub original_path: BlobPath,
    /// Its parent folder before it was trashed.
    pub original_parent: Option<ObjectId>,
}

/// A folder, page, book, collection or shortcut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The front-matter `id`: the only identity.
    pub id: ObjectId,
    /// The space it lives in.
    pub space: SpaceId,
    /// What it is.
    pub kind: NodeKind,
    /// Its parent folder, `None` at the space root.
    pub parent: Option<ObjectId>,
    /// The file (page, shortcut) or directory (folder, book, collection).
    pub path: BlobPath,
    /// Display title.
    pub title: String,
    /// Content size in bytes (all files, for books and collections).
    pub size: u64,
    /// Last content or location change.
    pub modified: Timestamp,
    /// Who created it, when known.
    pub created_by: Option<UserId>,
    /// For shortcuts: the node they point at.
    pub target: Option<ObjectId>,
    /// Set while the node is in the trash.
    pub trash: Option<TrashState>,
}

impl Node {
    /// True unless the node is in the trash.
    #[must_use]
    pub const fn is_live(&self) -> bool {
        self.trash.is_none()
    }
}

/// One recorded version of a page's content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRecord {
    /// Identity; its timestamp is when the version was written.
    pub id: VersionId,
    /// The page.
    pub doc: ObjectId,
    /// The page's space, for quota accounting.
    pub space: SpaceId,
    /// Size of the content in bytes.
    pub size: u64,
    /// Who wrote it.
    pub author: Actor,
}

impl VersionRecord {
    /// When the version was written, in Unix milliseconds.
    #[must_use]
    pub const fn written_ms(&self) -> u64 {
        self.id.timestamp_ms()
    }
}

/// What happened in an activity entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ActivityKind {
    /// The node was created.
    Created,
    /// The page's content changed.
    Edited {
        /// The version written.
        version: VersionId,
    },
    /// The node was renamed.
    Renamed {
        /// Old title.
        from: String,
        /// New title.
        to: String,
    },
    /// The node moved.
    Moved {
        /// Old path.
        from: String,
        /// New path.
        to: String,
    },
    /// The node was created as a copy of another.
    Copied {
        /// The original.
        from: ObjectId,
    },
    /// The node was moved to the trash.
    Trashed,
    /// The node came back from the trash.
    Restored,
    /// The node was deleted for good.
    Purged,
    /// A save conflicted and the writer's version went to a copy.
    ConflictCopy {
        /// The conflict copy.
        copy: ObjectId,
    },
    /// An earlier version was restored as the current content.
    VersionRestored {
        /// The version restored.
        version: VersionId,
    },
}

/// One entry in the activity feed of a node and its ancestors (R-DR-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Activity {
    /// Identity, time-ordered.
    pub id: EventId,
    /// When it happened.
    pub at: Timestamp,
    /// Who did it.
    pub actor: Actor,
    /// The space.
    pub space: SpaceId,
    /// The node it happened to.
    pub node: ObjectId,
    /// The node's folders at the time, innermost first, so a folder's feed
    /// includes everything beneath it.
    pub ancestors: Vec<ObjectId>,
    /// What happened.
    pub kind: ActivityKind,
}

/// A space's storage use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// Bytes used by content, trash and versions.
    pub used: u64,
    /// The quota, if any.
    pub quota: Option<u64>,
}

/// What a scan of the store changed in the index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Spaces found.
    pub spaces: usize,
    /// Nodes seen for the first time.
    pub added: usize,
    /// Nodes found at a new path (renamed or moved outside Vellum).
    pub relocated: usize,
    /// Nodes whose files are gone.
    pub removed: usize,
    /// Files given a fresh ID because another file already had theirs.
    pub reidentified: usize,
    /// Files skipped because they could not be read.
    pub skipped: usize,
}
