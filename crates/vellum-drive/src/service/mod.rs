// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! [`DriveService`]: the drive's operations as the gateway serves them,
//! one REST route, OpenAPI entry and `GraphQL` field each (D146).
//!
//! Every operation maps to one [`Drive`] method, so the drive's own
//! permission checks decide what the caller may do. The gateway checks
//! the credential's scope first: reads need `read`, changes need
//! `write:direct`.

pub mod api;

use std::sync::Arc;

use bytes::Bytes;
use vellum_contracts::{
    BoxFuture, HttpMethod, Operation, Readiness, RequestContext, Scope, Service,
};
use vellum_kernel::{Actor, Error, ObjectId, SpaceId, UserId};
use vellum_store::{Base, Etag};

use self::api::{
    ActivityInput, ActivityList, ActivityView, ContentView, CreateFolder, CreatePage,
    CreateShortcut, CreateSpace, Done, FindInput, Limit, ListChildren, MyDrive, NoInput, NodeKey,
    NodeList, NodeView, PlaceInput, RecentItem, RecentList, Removed, RenameInput, SaveInput,
    SaveResult, SaveView, SpaceKey, SpaceKindView, SpaceList, SpaceView, StarInput, TreeInput,
    TreeView, UpdateSpace, UsageView, VersionContent, VersionKey, VersionList, VersionView,
};
use crate::drive::{Drive, Filter, SaveOutcome, Saved};

/// The most results a list returns.
const MAX_LIMIT: usize = 200;
/// Results a list returns when the caller does not say.
const DEFAULT_LIMIT: usize = 50;
/// Items a tree returns when the caller does not say.
const DEFAULT_TREE_LIMIT: usize = 2_000;

/// Looks up a person's handle, for naming the My Drive made for them.
pub type Handles = Arc<dyn Fn(UserId) -> Option<String> + Send + Sync>;

/// Told when a page's text is written through the API: a save, or a
/// version restored. Live editing implements it so a page open in the
/// editor takes the new text instead of writing over it later (D172).
pub trait PageWrites: Send + Sync + std::fmt::Debug {
    /// The page's content changed. Called after the write; must not block.
    fn page_written(&self, space: SpaceId, page: ObjectId);
}

/// Serves the drive over the gateway.
#[derive(Clone)]
pub struct DriveService {
    drive: Arc<Drive>,
    handles: Option<Handles>,
    writes: Option<Arc<dyn PageWrites>>,
    sharing: Option<Arc<crate::sharing::Sharing>>,
}

impl std::fmt::Debug for DriveService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriveService")
            .field("drive", &self.drive)
            .field("handles", &self.handles.is_some())
            .field("writes", &self.writes)
            .field("sharing", &self.sharing.is_some())
            .finish()
    }
}

impl DriveService {
    /// Wraps the drive.
    #[must_use]
    pub const fn new(drive: Arc<Drive>) -> Self {
        Self {
            drive,
            handles: None,
            writes: None,
            sharing: None,
        }
    }

    /// Names the My Drive made on a person's first visit after their
    /// handle (D171); without it the slug is `me` or their ID.
    #[must_use]
    pub fn with_handles(mut self, handles: Handles) -> Self {
        self.handles = Some(handles);
        self
    }

    /// Serves sharing (#95); without it the sharing operations answer
    /// `unavailable`.
    #[must_use]
    pub fn with_sharing(mut self, sharing: Arc<crate::sharing::Sharing>) -> Self {
        self.sharing = Some(sharing);
        self
    }

    /// Tells `writes` about every page text written through the API.
    #[must_use]
    pub fn with_page_writes(mut self, writes: Arc<dyn PageWrites>) -> Self {
        self.writes = Some(writes);
        self
    }

    /// Passes a written page on, unless the save went to a conflict copy
    /// and left the page as it was.
    fn written(&self, outcome: &SaveOutcome) {
        if let Some(writes) = &self.writes
            && !matches!(outcome.saved, Saved::Conflicted { .. })
        {
            writes.page_written(outcome.node.space, outcome.node.id);
        }
    }
}

/// The signed-in person behind a request. Share links and anonymous
/// callers have no drive.
fn actor(ctx: &RequestContext) -> Result<Actor, Error> {
    Ok(Actor::User {
        user: ctx.principal.require_user()?,
    })
}

fn limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn text(bytes: &Bytes) -> Option<String> {
    String::from_utf8(bytes.to_vec()).ok()
}

fn saved(outcome: &SaveOutcome) -> SaveView {
    let (result, version_id, conflict_copy) = match &outcome.saved {
        Saved::Written(v) => (SaveResult::Written, Some(*v), None),
        Saved::Merged(v) => (SaveResult::Merged, Some(*v), None),
        Saved::Conflicted { copy } => (
            SaveResult::Conflicted,
            None,
            Some(NodeView::from(copy.as_ref())),
        ),
    };
    SaveView {
        item: NodeView::from(&outcome.node),
        result,
        version_id,
        conflict_copy,
    }
}

const fn done() -> Done {
    Done { ok: true }
}

/// Declares an operation whose handler is a `DriveService` method.
macro_rules! op {
    ($service:expr, query $id:literal, $path:literal, $method:ident, $summary:literal) => {{
        let s = Arc::clone(&$service);
        Operation::query($id, $path, Scope::Read, move |ctx, input| {
            let s = Arc::clone(&s);
            async move { s.$method(&ctx, input).await }
        })
        .summary($summary)
    }};
    ($service:expr, $verb:ident $id:literal, $path:literal, $method:ident, $summary:literal) => {{
        let s = Arc::clone(&$service);
        Operation::mutation(
            $id,
            HttpMethod::$verb,
            $path,
            Scope::WriteDirect,
            move |ctx, input| {
                let s = Arc::clone(&s);
                async move { s.$method(&ctx, input).await }
            },
        )
        .summary($summary)
    }};
}

pub mod sharing;

/// Reads.
impl DriveService {
    async fn my_drive(&self, ctx: &RequestContext, _: NoInput) -> Result<MyDrive, Error> {
        let user = ctx.principal.require_user()?;
        // Normally made with the account; made here if that was missed.
        let handle = self.handles.as_ref().and_then(|h| h(user));
        let space = self.drive.ensure_my_drive(user, handle.as_deref()).await?;
        Ok(MyDrive {
            space: Some(SpaceView::from(&space)),
        })
    }

    async fn shared_drives(&self, ctx: &RequestContext, _: NoInput) -> Result<SpaceList, Error> {
        let user = ctx.principal.require_user()?;
        let spaces = self.drive.shared_drives(user).await?;
        Ok(SpaceList {
            spaces: spaces.iter().map(SpaceView::from).collect(),
        })
    }

    async fn shared_with_me(&self, ctx: &RequestContext, _: NoInput) -> Result<NodeList, Error> {
        let user = ctx.principal.require_user()?;
        Ok(NodeList::of(&self.drive.shared_with_me(user).await?))
    }

    async fn list(&self, ctx: &RequestContext, input: ListChildren) -> Result<NodeList, Error> {
        let nodes = self
            .drive
            .list(&actor(ctx)?, input.space_id, input.parent_id)
            .await?;
        Ok(NodeList::of(&nodes))
    }

    async fn tree(&self, ctx: &RequestContext, input: TreeInput) -> Result<TreeView, Error> {
        let tree = self
            .drive
            .tree(
                &actor(ctx)?,
                input.space_id,
                input.root_id,
                input.depth,
                input.limit.unwrap_or(DEFAULT_TREE_LIMIT),
            )
            .await?;
        Ok(TreeView {
            items: tree.items.iter().map(NodeView::from).collect(),
            truncated: tree.truncated,
        })
    }

    async fn get(&self, ctx: &RequestContext, input: NodeKey) -> Result<NodeView, Error> {
        Ok(NodeView::from(
            &self.drive.get(&actor(ctx)?, input.id).await?,
        ))
    }

    async fn read(&self, ctx: &RequestContext, input: NodeKey) -> Result<ContentView, Error> {
        let (node, blob) = self.drive.read(&actor(ctx)?, input.id).await?;
        Ok(ContentView {
            item: NodeView::from(&node),
            content: text(&blob.bytes),
            etag: blob.meta.etag.to_string(),
        })
    }

    async fn resolve_shortcut(
        &self,
        ctx: &RequestContext,
        input: NodeKey,
    ) -> Result<NodeView, Error> {
        let node = self.drive.resolve_shortcut(&actor(ctx)?, input.id).await?;
        Ok(NodeView::from(&node))
    }

    async fn versions(&self, ctx: &RequestContext, input: NodeKey) -> Result<VersionList, Error> {
        let versions = self.drive.versions(&actor(ctx)?, input.id).await?;
        Ok(VersionList {
            versions: versions.iter().map(VersionView::from).collect(),
        })
    }

    async fn version(
        &self,
        ctx: &RequestContext,
        input: VersionKey,
    ) -> Result<VersionContent, Error> {
        let bytes = self
            .drive
            .version_content(&actor(ctx)?, input.id, input.version_id)
            .await?;
        Ok(VersionContent {
            id: input.version_id,
            content: text(&bytes),
        })
    }

    async fn starred(&self, ctx: &RequestContext, _: NoInput) -> Result<NodeList, Error> {
        Ok(NodeList::of(&self.drive.starred(&actor(ctx)?).await?))
    }

    async fn recent(&self, ctx: &RequestContext, input: Limit) -> Result<RecentList, Error> {
        let recent = self.drive.recent(&actor(ctx)?, limit(input.limit)).await?;
        Ok(RecentList {
            items: recent
                .iter()
                .map(|(node, at)| RecentItem {
                    item: NodeView::from(node),
                    opened_at: *at,
                })
                .collect(),
        })
    }

    async fn activity(
        &self,
        ctx: &RequestContext,
        input: ActivityInput,
    ) -> Result<ActivityList, Error> {
        let events = self
            .drive
            .activity(&actor(ctx)?, input.id, limit(input.limit))
            .await?;
        Ok(ActivityList {
            events: events.iter().map(ActivityView::from).collect(),
        })
    }

    async fn backlinks(&self, ctx: &RequestContext, input: NodeKey) -> Result<NodeList, Error> {
        Ok(NodeList::of(
            &self.drive.backlinks(&actor(ctx)?, input.id).await?,
        ))
    }

    async fn usage(&self, ctx: &RequestContext, input: SpaceKey) -> Result<UsageView, Error> {
        let usage = self.drive.usage(&actor(ctx)?, input.space_id).await?;
        Ok(UsageView {
            used_bytes: usage.used,
            quota_bytes: usage.quota,
        })
    }

    async fn trash_list(&self, ctx: &RequestContext, input: SpaceKey) -> Result<NodeList, Error> {
        let nodes = self.drive.trash_list(&actor(ctx)?, input.space_id).await?;
        Ok(NodeList::of(&nodes))
    }

    async fn find(&self, ctx: &RequestContext, input: FindInput) -> Result<NodeList, Error> {
        let filter = Filter {
            title: input.title,
            owner: input.created_by,
            modified_after: input.modified_after,
            modified_before: input.modified_before,
            kinds: input.kinds.into_iter().map(Into::into).collect(),
            location: input.location,
            starred: input.starred,
        };
        let nodes = self
            .drive
            .find(&actor(ctx)?, &filter, limit(input.limit))
            .await?;
        Ok(NodeList::of(&nodes))
    }
}

/// Changes.
impl DriveService {
    async fn create_space(
        &self,
        ctx: &RequestContext,
        input: CreateSpace,
    ) -> Result<SpaceView, Error> {
        let user = ctx.principal.require_user()?;
        let owner = (input.kind == SpaceKindView::UserDrive).then_some(user);
        let space = self
            .drive
            .create_space(
                &actor(ctx)?,
                input.kind.into(),
                &input.slug,
                &input.name,
                owner,
            )
            .await?;
        Ok(SpaceView::from(&space))
    }

    async fn update_space(
        &self,
        ctx: &RequestContext,
        input: UpdateSpace,
    ) -> Result<SpaceView, Error> {
        let space = self
            .drive
            .update_space(
                &actor(ctx)?,
                input.space_id,
                input.name.as_deref(),
                input.settings.into(),
            )
            .await?;
        Ok(SpaceView::from(&space))
    }

    async fn create_folder(
        &self,
        ctx: &RequestContext,
        input: CreateFolder,
    ) -> Result<NodeView, Error> {
        let node = self
            .drive
            .create_folder(&actor(ctx)?, input.space_id, input.parent_id, &input.name)
            .await?;
        Ok(NodeView::from(&node))
    }

    async fn create_page(
        &self,
        ctx: &RequestContext,
        input: CreatePage,
    ) -> Result<NodeView, Error> {
        let node = self
            .drive
            .create_page(
                &actor(ctx)?,
                input.space_id,
                input.parent_id,
                &input.title,
                &input.body,
            )
            .await?;
        Ok(NodeView::from(&node))
    }

    async fn create_shortcut(
        &self,
        ctx: &RequestContext,
        input: CreateShortcut,
    ) -> Result<NodeView, Error> {
        let node = self
            .drive
            .create_shortcut(
                &actor(ctx)?,
                input.target_id,
                input.space_id,
                input.parent_id,
            )
            .await?;
        Ok(NodeView::from(&node))
    }

    async fn save(&self, ctx: &RequestContext, input: SaveInput) -> Result<SaveView, Error> {
        let base = Base {
            bytes: Bytes::from(input.base_content),
            etag: Some(Etag::new(input.base_etag)),
        };
        let outcome = self
            .drive
            .save(&actor(ctx)?, input.id, base, Bytes::from(input.content))
            .await?;
        self.written(&outcome);
        Ok(saved(&outcome))
    }

    async fn restore_version(
        &self,
        ctx: &RequestContext,
        input: VersionKey,
    ) -> Result<SaveView, Error> {
        let outcome = self
            .drive
            .restore_version(&actor(ctx)?, input.id, input.version_id)
            .await?;
        self.written(&outcome);
        Ok(saved(&outcome))
    }

    async fn rename(&self, ctx: &RequestContext, input: RenameInput) -> Result<NodeView, Error> {
        let node = self
            .drive
            .rename(&actor(ctx)?, input.id, &input.title)
            .await?;
        Ok(NodeView::from(&node))
    }

    async fn move_node(&self, ctx: &RequestContext, input: PlaceInput) -> Result<NodeView, Error> {
        let actor = actor(ctx)?;
        let mut node = self
            .drive
            .move_node(&actor, input.id, input.space_id, input.parent_id)
            .await?;
        if let Some(index) = input.index {
            node = self.drive.reorder(&actor, node.id, index).await?;
        }
        Ok(NodeView::from(&node))
    }

    async fn copy_node(&self, ctx: &RequestContext, input: PlaceInput) -> Result<NodeView, Error> {
        let actor = actor(ctx)?;
        let mut node = self
            .drive
            .copy_node(&actor, input.id, input.space_id, input.parent_id)
            .await?;
        if let Some(index) = input.index {
            node = self.drive.reorder(&actor, node.id, index).await?;
        }
        Ok(NodeView::from(&node))
    }

    async fn trash(&self, ctx: &RequestContext, input: NodeKey) -> Result<NodeView, Error> {
        Ok(NodeView::from(
            &self.drive.trash(&actor(ctx)?, input.id).await?,
        ))
    }

    async fn restore(&self, ctx: &RequestContext, input: NodeKey) -> Result<NodeView, Error> {
        Ok(NodeView::from(
            &self.drive.restore(&actor(ctx)?, input.id).await?,
        ))
    }

    async fn purge(&self, ctx: &RequestContext, input: NodeKey) -> Result<Done, Error> {
        self.drive.purge(&actor(ctx)?, input.id).await?;
        Ok(done())
    }

    async fn empty_trash(&self, ctx: &RequestContext, input: SpaceKey) -> Result<Removed, Error> {
        let removed = self.drive.empty_trash(&actor(ctx)?, input.space_id).await?;
        Ok(Removed { removed })
    }

    async fn prune_versions(
        &self,
        ctx: &RequestContext,
        input: SpaceKey,
    ) -> Result<Removed, Error> {
        let removed = self
            .drive
            .prune_versions(&actor(ctx)?, input.space_id)
            .await?;
        Ok(Removed { removed })
    }

    async fn star(&self, ctx: &RequestContext, input: StarInput) -> Result<Done, Error> {
        self.drive
            .star(&actor(ctx)?, input.id, input.starred)
            .await?;
        Ok(done())
    }
}

impl DriveService {
    fn reads(self: &Arc<Self>) -> Vec<Operation> {
        vec![
            op!(self, query "drive.my_drive", "/v1/drive/my-drive", my_drive,
                "Get your My Drive, made for you if you have none yet"),
            op!(self, query "drive.shared_drives", "/v1/drive/shared-drives", shared_drives,
                "List the shared drives you can open"),
            op!(self, query "drive.shared_with_me", "/v1/drive/shared-with-me", shared_with_me,
                "List items in other people's My Drives shared with you"),
            op!(self, query "drive.list", "/v1/drive/spaces/{space_id}/items", list,
                "List the items in a folder or at the top of a space"),
            op!(self, query "drive.tree", "/v1/drive/spaces/{space_id}/tree", tree,
                "List every item in a space, or under one item, in tree order"),
            op!(self, query "drive.get", "/v1/drive/items/{id}", get, "Get an item"),
            op!(self, query "drive.read", "/v1/drive/items/{id}/content", read,
                "Read a page's content and its etag"),
            op!(self, query "drive.resolve_shortcut", "/v1/drive/items/{id}/target",
                resolve_shortcut, "Get the item a shortcut points at"),
            op!(self, query "drive.versions", "/v1/drive/items/{id}/versions", versions,
                "List a page's saved versions"),
            op!(self, query "drive.version", "/v1/drive/items/{id}/versions/{version_id}", version,
                "Read one saved version of a page"),
            op!(self, query "drive.starred", "/v1/drive/starred", starred,
                "List the items you starred"),
            op!(self, query "drive.recent", "/v1/drive/recent", recent,
                "List the items you opened recently"),
            op!(self, query "drive.activity", "/v1/drive/items/{id}/activity", activity,
                "List the changes to an item, or to a whole space by its ID"),
            op!(self, query "drive.backlinks", "/v1/drive/items/{id}/backlinks", backlinks,
                "List the pages that link to a page"),
            op!(self, query "drive.usage", "/v1/drive/spaces/{space_id}/usage", usage,
                "Get how much of its quota a space uses"),
            op!(self, query "drive.trash_list", "/v1/drive/spaces/{space_id}/trash", trash_list,
                "List what was trashed in a space"),
            op!(self, query "drive.find", "/v1/drive/search", find,
                "Find items by title, creator, date, kind, place or star"),
        ]
    }

    fn changes(self: &Arc<Self>) -> Vec<Operation> {
        vec![
            op!(self, Post "drive.create_space", "/v1/drive/spaces", create_space,
                "Create a shared drive, or your My Drive"),
            op!(self, Patch "drive.update_space", "/v1/drive/spaces/{space_id}", update_space,
                "Rename a space and change its quota, trash and version settings"),
            op!(self, Post "drive.create_folder", "/v1/drive/spaces/{space_id}/folders",
                create_folder, "Create a folder"),
            op!(self, Post "drive.create_page", "/v1/drive/spaces/{space_id}/pages", create_page,
                "Create a page"),
            op!(self, Post "drive.create_shortcut", "/v1/drive/spaces/{space_id}/shortcuts",
                create_shortcut, "Create a shortcut to another item"),
            op!(self, Put "drive.save", "/v1/drive/items/{id}/content", save,
                "Save a page, merging changes made meanwhile"),
            op!(self, Post "drive.restore_version",
                "/v1/drive/items/{id}/versions/{version_id}/restore", restore_version,
                "Make an old version the page's content again"),
            op!(self, Patch "drive.rename", "/v1/drive/items/{id}", rename,
                "Rename an item and fix the links to it"),
            op!(self, Post "drive.move", "/v1/drive/items/{id}/move", move_node,
                "Move an item to another folder or space"),
            op!(self, Post "drive.copy", "/v1/drive/items/{id}/copy", copy_node,
                "Copy an item, with fresh IDs, into a folder or space"),
            op!(self, Post "drive.trash", "/v1/drive/items/{id}/trash", trash,
                "Move an item, with everything in it, to the trash"),
            op!(self, Post "drive.restore", "/v1/drive/items/{id}/restore", restore,
                "Bring a trashed item back"),
            op!(self, Delete "drive.purge", "/v1/drive/items/{id}", purge,
                "Delete a trashed item for good"),
            op!(self, Delete "drive.empty_trash", "/v1/drive/spaces/{space_id}/trash",
                empty_trash, "Delete everything in a space's trash for good"),
            op!(self, Post "drive.prune_versions", "/v1/drive/spaces/{space_id}/prune-versions",
                prune_versions, "Apply the space's version retention now"),
            op!(self, Put "drive.star", "/v1/drive/items/{id}/star", star,
                "Star or unstar an item"),
        ]
    }
}

impl Service for DriveService {
    fn name(&self) -> &'static str {
        "drive"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let mut ops = self.reads();
        ops.extend(self.changes());
        ops.extend(self.sharing_ops());
        ops
    }

    fn readiness(&self) -> BoxFuture<'_, Readiness> {
        Box::pin(async move {
            match self.drive.ready().await {
                Ok(()) => Readiness::Ready,
                Err(err) => Readiness::NotReady(format!("the drive index is unavailable: {err}")),
            }
        })
    }
}
