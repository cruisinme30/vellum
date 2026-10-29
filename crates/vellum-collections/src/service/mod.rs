// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The collections service: the engine behind the gateway.
//!
//! [`CollectionsService`] implements [`Service`] over two ports: a
//! [`BlobStore`] holding the collection folders and an [`Access`] check
//! for per-object permissions. It loads every collection on first use,
//! serves reads from memory, and writes row pages back to the store on
//! each edit. Call [`CollectionsService::invalidate`] when the watcher
//! sees a collection file change on disk.
//!
//! | Operation | Route | Scope |
//! |---|---|---|
//! | `collections.list` | `GET /v1/collections` | `read` |
//! | `collections.get` | `GET /v1/collections/{collection_id}` | `read` |
//! | `collections.query` | `GET /v1/collections/{collection_id}/query` | `read` |
//! | `collections.get_row` | `GET /v1/collections/{collection_id}/rows/{row_id}` | `read` |
//! | `collections.create_row` | `POST /v1/collections/{collection_id}/rows` | `write:direct` |
//! | `collections.update_row` | `PATCH /v1/collections/{collection_id}/rows/{row_id}` | `write:direct` |
//! | `collections.delete_row` | `DELETE /v1/collections/{collection_id}/rows/{row_id}` | `write:direct` |
//! | `collections.save_view` | `PUT /v1/collections/{collection_id}/views/{view}` | `write:direct` |
//! | `collections.delete_view` | `DELETE /v1/collections/{collection_id}/views/{view}` | `write:direct` |
//! | `collections.embed` | `GET /v1/collections/embed` | `read` |
//! | `collections.chart` | `GET /v1/collections/chart` | `read` |
//! | `collections.export_csv` | `GET /v1/collections/{collection_id}/csv` | `read` |
//! | `collections.import_csv` | `POST /v1/collections/{collection_id}/csv` | `write:direct` |
//! | `collections.create_from_csv` | `POST /v1/collections` | `write:direct` |
//!
//! Reading a collection needs read access to it and to every collection
//! its relations point at, because relation cells show related titles
//! and rollups show related values. Editing also needs edit access to the
//! other side of each two-way relation, because the engine mirrors links
//! there.

pub mod access;
pub mod api;
pub mod index;
mod state;

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::sync::{RwLock, RwLockReadGuard};
use tracing::Instrument as _;
use vellum_contracts::{
    BoxFuture, HttpMethod, Operation, Principal, Readiness, RequestContext, Scope, Service,
};
use vellum_kernel::{CollectionId, DocId, Error, ErrorKind};
use vellum_store::BlobStore;

pub use access::{Access, AuthzAccess, Need, SpaceOf};
pub use api::{
    ChartInput, CollectionInfo, CollectionKey, CollectionList, CollectionSummary,
    CreateFromCsvInput, CreateRowInput, CsvExport, CsvImported, EmbedInput, EmbedResult,
    ExportCsvInput, ImportCsvInput, ListCollections, PropertyInfo, QueryInput, RelationInfo,
    RollupInfo, RowDetail, RowKey, RowWritten, SaveViewInput, UpdateRowInput, ViewInfo, ViewKey,
};
pub use index::{IndexFeed, IndexField, RowDocument};
pub use state::{MAX_FILE_BYTES, MAX_ROWS_PER_COLLECTION, collection_dir};

/// Largest CSV file accepted for import, in bytes.
pub const MAX_CSV_BYTES: usize = 16 * 1024 * 1024;

/// The saved view `name`, the first view, or a plain table.
fn view_def(schema: &crate::schema::Schema, name: Option<&str>) -> Result<ViewDef, Error> {
    match (name, schema.view(name)) {
        (_, Some(v)) => Ok(v.clone()),
        (None, None) => Ok(ViewDef::default_table()),
        (Some(name), None) => Err(Error::new(
            ErrorKind::NotFound,
            format!("no view named {name:?}"),
        )),
    }
}

fn csv_size(text: &str) -> Result<(), Error> {
    if text.len() > MAX_CSV_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!(
                "CSV files over {} MiB cannot be imported",
                MAX_CSV_BYTES / 1024 / 1024
            ),
        ));
    }
    Ok(())
}

/// Most saved views one collection may have.
pub const MAX_VIEWS: usize = 64;
/// Longest view name, in characters.
pub const MAX_VIEW_NAME_CHARS: usize = 100;

fn one_or_many(items: &[String]) -> OneOrMany {
    match items {
        [] => OneOrMany::None,
        [one] => OneOrMany::One(one.clone()),
        many => OneOrMany::Many(many.to_vec()),
    }
}

fn view_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    if name.is_empty()
        || name.chars().count() > MAX_VIEW_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("a view name needs 1 to {MAX_VIEW_NAME_CHARS} printable characters"),
        ));
    }
    Ok(name.to_owned())
}

fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// A CSV import matched against the loaded collections and, unless it is
/// a dry run, applied to a copy of them.
struct PreparedImport {
    import: crate::csv::CsvImport,
    ws: Option<crate::workspace::Workspace>,
    changed: crate::workspace::Changeset,
}

impl PreparedImport {
    fn report(self, written: bool, collection: CollectionInfo) -> CsvImported {
        CsvImported {
            written,
            rows: self.import.rows.len(),
            skipped_columns: self.import.skipped_columns,
            diagnostics: self.import.diagnostics,
            collection,
            changed: self.changed,
        }
    }
}

/// Matches `csv` against collection `id` and applies it to a copy of the
/// workspace, so a bad row leaves nothing half done.
fn prepare_import(
    state: &State,
    csv: &str,
    id: CollectionId,
    dry_run: bool,
) -> Result<PreparedImport, Error> {
    let import = crate::csv::import_into(csv, &state.ws, id)?;
    let mut changed = crate::workspace::Changeset::default();
    if dry_run {
        return Ok(PreparedImport {
            import,
            ws: None,
            changed,
        });
    }
    let mut ws = state.ws.clone();
    for edit in import.edits() {
        let c = ws.apply(edit)?;
        changed.written.extend(c.written);
        changed.deleted.extend(c.deleted);
    }
    Ok(PreparedImport {
        import,
        ws: Some(ws),
        changed,
    })
}

/// A loaded collection's schema, views and load problems.
fn collection_info(state: &State, id: CollectionId) -> Result<CollectionInfo, Error> {
    match (state.ws.collection(id), state.folder(id)) {
        (Some(c), Ok(f)) => Ok(CollectionInfo::new(
            c.schema(),
            f.path.as_str().to_owned(),
            f.diagnostics.clone(),
        )),
        _ => Err(Error::new(ErrorKind::Internal, "collection vanished")),
    }
}

use crate::compute::Computer;
use crate::value::DateValue;
use crate::view::{self, OneOrMany, ViewDef, ViewDefRaw, ViewResult};
use crate::workspace::Edit;
use state::State;

/// The current time, as formulas see it (`now()`, `today()`).
pub type Clock = Arc<dyn Fn() -> DateValue + Send + Sync>;

fn system_now() -> DateValue {
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    DateValue::from_unix_ms(ms)
        .or_else(|| DateValue::ymd(1970, 1, 1))
        .unwrap_or_else(|| unreachable!("1970-01-01 is a valid date"))
}

/// Serves collections over the gateway.
pub struct CollectionsService {
    store: Arc<dyn BlobStore>,
    access: Arc<dyn Access>,
    clock: Clock,
    state: RwLock<Option<State>>,
    /// Bumped under the write lock by every change to `state`, so work
    /// prepared under the read lock can tell whether it is still current.
    writes: AtomicU64,
    index_feed: Option<Arc<dyn IndexFeed>>,
}

impl fmt::Debug for CollectionsService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CollectionsService")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

impl CollectionsService {
    /// A service over `store`, checking every request with `access`.
    #[must_use]
    pub fn new(store: Arc<dyn BlobStore>, access: Arc<dyn Access>) -> Self {
        Self {
            store,
            access,
            clock: Arc::new(system_now),
            state: RwLock::new(None),
            writes: AtomicU64::new(0),
            index_feed: None,
        }
    }

    /// Tells `feed` which rows to index again after every write, so search
    /// sees computed values (formulas, rollups) as they change.
    #[must_use]
    pub fn with_index_feed(mut self, feed: Arc<dyn IndexFeed>) -> Self {
        self.index_feed = Some(feed);
        self
    }

    fn feed(&self, rows: Vec<DocId>) {
        if let Some(feed) = &self.index_feed
            && !rows.is_empty()
        {
            feed.rows_changed(rows);
        }
    }

    /// A row as search should see it, computed values included; `None`
    /// when no collection has it (deleted, or never a row). No access
    /// check: this is for the indexer, and search filters every hit by
    /// what the reader may open.
    ///
    /// # Errors
    ///
    /// When the collections cannot be read from the store.
    pub async fn index_row(&self, row: DocId) -> Result<Option<RowDocument>, Error> {
        let state = self.read().await?;
        let mut computer = Computer::new(&state.ws, (self.clock)());
        let doc = index::row_document(&state, &mut computer, row);
        drop(computer);
        drop(state);
        Ok(doc)
    }

    /// Every row, for a full rebuild of the search index.
    ///
    /// # Errors
    ///
    /// When the collections cannot be read from the store.
    pub async fn index_rows(&self) -> Result<Vec<DocId>, Error> {
        let state = self.read().await?;
        Ok(index::all_rows(&state))
    }

    /// Replaces the clock, for tests and reproducible exports.
    #[must_use]
    pub fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// Drops everything loaded; the next request reads the store again.
    /// The watcher calls this when a collection file changes on disk.
    pub async fn invalidate(&self) {
        let mut guard = self.state.write().await;
        self.bump();
        *guard = None;
    }

    /// Re-reads the collection folders in `dirs` after another replica
    /// wrote to them (ADR 0038), keeping every other collection loaded.
    /// Falls back to [`Self::invalidate`] when a targeted reload cannot
    /// give the same result as a full one.
    pub async fn refresh(&self, dirs: &std::collections::BTreeSet<vellum_store::BlobPath>) {
        if dirs.is_empty() {
            return;
        }
        let mut guard = self.state.write().await;
        self.bump();
        let Some(state) = guard.as_mut() else {
            // Nothing loaded yet: the next request reads everything fresh.
            return;
        };
        if let Err(error) = state.reload(self.store.as_ref(), dirs).await {
            tracing::info!(%error, "reloading every collection after a change elsewhere");
            *guard = None;
        }
    }

    /// Marks the loaded state as changed. Call with the write lock held.
    fn bump(&self) {
        let _ = self.writes.fetch_add(1, Ordering::AcqRel);
    }

    async fn read(&self) -> Result<RwLockReadGuard<'_, State>, Error> {
        {
            let guard = self.state.read().await;
            if let Ok(state) = RwLockReadGuard::try_map(guard, Option::as_ref) {
                return Ok(state);
            }
        }
        let mut guard = self.state.write().await;
        if guard.is_none() {
            *guard = Some(State::load(self.store.as_ref()).await?);
        }
        RwLockReadGuard::try_map(guard.downgrade(), Option::as_ref)
            .map_err(|_| Error::new(ErrorKind::Internal, "collections were not loaded"))
    }

    /// Checks that `principal` may read `id`, answering "not found" when it
    /// may not, so a caller cannot probe for collections it cannot see.
    async fn require_read(
        &self,
        state: &State,
        principal: &Principal,
        id: CollectionId,
    ) -> Result<(), Error> {
        let not_found = || Error::new(ErrorKind::NotFound, format!("collection {id} not found"));
        let folder = state.folder(id).map_err(|_| not_found())?;
        if !self
            .access
            .allowed(principal, id, &folder.path, Need::Read)
            .await?
        {
            return Err(not_found());
        }
        for other in state.read_set(id) {
            if other == id {
                continue;
            }
            let Ok(f) = state.folder(other) else {
                continue;
            };
            if !self
                .access
                .allowed(principal, other, &f.path, Need::Read)
                .await?
            {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    "this collection links to a collection you cannot read",
                ));
            }
        }
        Ok(())
    }

    /// Checks edit access to `id`, and with `mirrored` also to the other
    /// side of each two-way relation (row edits write there too).
    async fn require_edit(
        &self,
        state: &State,
        principal: &Principal,
        id: CollectionId,
        mirrored: bool,
    ) -> Result<(), Error> {
        self.require_read(state, principal, id).await?;
        let targets = if mirrored {
            state.write_set(id)
        } else {
            std::collections::BTreeSet::from([id])
        };
        for target in targets {
            let Ok(f) = state.folder(target) else {
                continue;
            };
            if !self
                .access
                .allowed(principal, target, &f.path, Need::Edit)
                .await?
            {
                let message = if target == id {
                    "you cannot edit this collection"
                } else {
                    "editing this collection also changes a linked collection you cannot edit"
                };
                return Err(Error::new(ErrorKind::PermissionDenied, message));
            }
        }
        Ok(())
    }

    /// The collections the caller can read, by title.
    ///
    /// # Errors
    ///
    /// When the store or the access check is unavailable.
    pub async fn list(&self, ctx: &RequestContext) -> Result<CollectionList, Error> {
        let state = self.read().await?;
        let mut collections = Vec::new();
        for c in state.ws.collections() {
            let Ok(folder) = state.folder(c.id()) else {
                continue;
            };
            if !self
                .access
                .allowed(&ctx.principal, c.id(), &folder.path, Need::Read)
                .await?
            {
                continue;
            }
            collections.push(CollectionSummary {
                id: c.id(),
                title: c.schema().title.clone(),
                path: folder.path.as_str().to_owned(),
                rows: c.len(),
                views: c.schema().views.iter().map(|v| v.name.clone()).collect(),
            });
        }
        drop(state);
        collections.sort_by(|a, b| {
            a.title
                .to_lowercase()
                .cmp(&b.title.to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(CollectionList { collections })
    }

    /// One collection's schema, views and load problems.
    ///
    /// # Errors
    ///
    /// `not_found` when it does not exist or the caller cannot read it.
    pub async fn get(
        &self,
        ctx: &RequestContext,
        id: CollectionId,
    ) -> Result<CollectionInfo, Error> {
        let state = self.read().await?;
        self.require_read(&state, &ctx.principal, id).await?;
        let folder = state.folder(id)?;
        let c = state
            .ws
            .collection(id)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("collection {id} not found")))?;
        let info = CollectionInfo::new(
            c.schema(),
            folder.path.as_str().to_owned(),
            folder.diagnostics.clone(),
        );
        drop(state);
        Ok(info)
    }

    /// Runs a view.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown collection or view, `invalid_input` for a
    /// bad ad-hoc filter or sort.
    pub async fn query(
        &self,
        ctx: &RequestContext,
        input: QueryInput,
    ) -> Result<ViewResult, Error> {
        let state = self.read().await?;
        let id = input.collection_id;
        self.require_read(&state, &ctx.principal, id).await?;
        let schema = state
            .ws
            .collection(id)
            .map(crate::workspace::Collection::schema)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("collection {id} not found")))?;
        let mut def = view_def(schema, input.view.as_deref())?;
        let list = one_or_many;
        let overlay = ViewDefRaw {
            name: Some(def.name.clone()),
            kind: input.kind,
            filter: list(&input.filter),
            sort: list(&input.sort),
            group: input.group.clone(),
            hide: list(&input.hide),
            date: None,
            end: None,
            limit: input.limit,
        };
        def.overlay(&overlay, schema)?;
        let mut computer = Computer::new(&state.ws, (self.clock)());
        let result = view::query(&mut computer, id, &def);
        drop(computer);
        drop(state);
        Ok(result?)
    }

    fn detail(&self, state: &State, id: CollectionId, row: DocId) -> Result<RowDetail, Error> {
        let not_found = || Error::new(ErrorKind::NotFound, format!("row {row} not found"));
        let c = state.ws.collection(id).ok_or_else(not_found)?;
        let r = c.row(row).ok_or_else(not_found)?;
        let path = state
            .folder(id)?
            .rows
            .get(&row)
            .map(|(p, _)| p.as_str().to_owned())
            .unwrap_or_default();
        let mut computer = Computer::new(&state.ws, (self.clock)());
        let cells = computer.row(id, row).ok_or_else(not_found)?;
        Ok(RowDetail {
            collection_id: id,
            path,
            title: r.title(c.schema()),
            row: cells,
            body: r.body.clone(),
        })
    }

    /// One row with its computed values.
    ///
    /// # Errors
    ///
    /// `not_found` when the collection or row does not exist or cannot be
    /// read.
    pub async fn get_row(&self, ctx: &RequestContext, key: RowKey) -> Result<RowDetail, Error> {
        let state = self.read().await?;
        self.require_read(&state, &ctx.principal, key.collection_id)
            .await?;
        self.detail(&state, key.collection_id, key.row_id)
    }

    /// Applies an edit and writes the files it touches. On a write failure
    /// the loaded state is dropped, so the next request sees the disk.
    async fn edit(
        &self,
        ctx: &RequestContext,
        op: &'static str,
        id: CollectionId,
        edit: Edit,
        row: Option<DocId>,
    ) -> Result<RowWritten, Error> {
        let span = tracing::info_span!("collections.edit", op, collection = %id);
        async move {
            let mut guard = self.state.write().await;
            self.bump();
            if guard.is_none() {
                *guard = Some(State::load(self.store.as_ref()).await?);
            }
            let Some(state) = guard.as_mut() else {
                return Err(Error::new(
                    ErrorKind::Internal,
                    "collections were not loaded",
                ));
            };
            self.require_edit(state, &ctx.principal, id, true).await?;
            let changed = state.ws.apply(edit)?;
            let result = state.write(self.store.as_ref(), &changed).await;
            crate::telemetry::wrote(op, result.is_ok());
            if let Err(e) = result {
                *guard = None;
                return Err(e);
            }
            self.feed(index::affected_rows(state, &changed));
            let row = row.map(|r| self.detail(state, id, r)).transpose();
            drop(guard);
            Ok(RowWritten { row: row?, changed })
        }
        .instrument(span)
        .await
    }

    /// Creates a row.
    ///
    /// # Errors
    ///
    /// `invalid_input` for an unknown property or a value of the wrong
    /// type; `permission_denied` without edit access.
    pub async fn create_row(
        &self,
        ctx: &RequestContext,
        input: CreateRowInput,
    ) -> Result<RowWritten, Error> {
        let id = DocId::generate();
        let edit = Edit::CreateRow {
            collection: input.collection_id,
            id: Some(id),
            values: input.values,
            body: input.body,
        };
        self.edit(ctx, "create_row", input.collection_id, edit, Some(id))
            .await
    }

    /// Changes some values of a row.
    ///
    /// # Errors
    ///
    /// As [`Self::create_row`], and `not_found` for an unknown row;
    /// `conflict` when the file changed on disk since it was read.
    pub async fn update_row(
        &self,
        ctx: &RequestContext,
        input: UpdateRowInput,
    ) -> Result<RowWritten, Error> {
        let edit = Edit::UpdateRow {
            collection: input.collection_id,
            row: input.row_id,
            values: input.values,
            body: input.body,
        };
        self.edit(
            ctx,
            "update_row",
            input.collection_id,
            edit,
            Some(input.row_id),
        )
        .await
    }

    /// Deletes a row, moving its page to the trash and unlinking it.
    ///
    /// # Errors
    ///
    /// As [`Self::update_row`].
    pub async fn delete_row(&self, ctx: &RequestContext, key: RowKey) -> Result<RowWritten, Error> {
        let edit = Edit::DeleteRow {
            collection: key.collection_id,
            row: key.row_id,
        };
        self.edit(ctx, "delete_row", key.collection_id, edit, None)
            .await
    }
}

impl CollectionsService {
    /// Changes a collection's schema under the write lock and writes
    /// `schema.yaml`; returns the collection as it now stands.
    async fn edit_schema<F>(
        &self,
        ctx: &RequestContext,
        op: &'static str,
        id: CollectionId,
        change: F,
    ) -> Result<CollectionInfo, Error>
    where
        F: FnOnce(&mut crate::schema::Schema) -> Result<(), Error> + Send,
    {
        let span = tracing::info_span!("collections.edit_schema", op, collection = %id);
        async move {
            let mut guard = self.state.write().await;
            self.bump();
            if guard.is_none() {
                *guard = Some(State::load(self.store.as_ref()).await?);
            }
            let Some(state) = guard.as_mut() else {
                return Err(Error::new(
                    ErrorKind::Internal,
                    "collections were not loaded",
                ));
            };
            self.require_edit(state, &ctx.principal, id, false).await?;
            let mut schema = state
                .ws
                .collection(id)
                .map(|c| c.schema().clone())
                .ok_or_else(|| {
                    Error::new(ErrorKind::NotFound, format!("collection {id} not found"))
                })?;
            change(&mut schema)?;
            state.ws.set_schema(schema)?;
            let result = state.write_schema(self.store.as_ref(), id).await;
            crate::telemetry::wrote(op, result.is_ok());
            if let Err(e) = result {
                *guard = None;
                return Err(e);
            }
            let info = match (state.ws.collection(id), state.folder(id)) {
                (Some(c), Ok(f)) => CollectionInfo::new(
                    c.schema(),
                    f.path.as_str().to_owned(),
                    f.diagnostics.clone(),
                ),
                _ => return Err(Error::new(ErrorKind::Internal, "collection vanished")),
            };
            drop(guard);
            Ok(info)
        }
        .instrument(span)
        .await
    }

    /// Saves a view: replaces the one with this name (and renames it when
    /// asked) or adds a new one at the end.
    ///
    /// # Errors
    ///
    /// `invalid_input` for a bad name, filter, sort or property;
    /// `conflict` when the new name is taken or `schema.yaml` changed on
    /// disk; `permission_denied` without edit access.
    pub async fn save_view(
        &self,
        ctx: &RequestContext,
        input: SaveViewInput,
    ) -> Result<CollectionInfo, Error> {
        let name = view_name(&input.view)?;
        let new_name = input.rename_to.as_deref().map(view_name).transpose()?;
        self.edit_schema(ctx, "save_view", input.collection_id, move |schema| {
            let raw = ViewDefRaw {
                name: Some(new_name.clone().unwrap_or_else(|| name.clone())),
                kind: Some(input.kind.unwrap_or_default()),
                filter: one_or_many(&input.filter),
                sort: one_or_many(&input.sort),
                group: input.group.clone(),
                hide: one_or_many(&input.hide),
                date: input.date.clone(),
                end: input.end.clone(),
                limit: input.limit,
            };
            let def = ViewDef::from_raw(&raw, schema)?;
            let at = schema.views.iter().position(|v| same_name(&v.name, &name));
            if let Some(new) = &new_name {
                let taken = schema
                    .views
                    .iter()
                    .enumerate()
                    .any(|(i, v)| Some(i) != at && same_name(&v.name, new));
                if taken {
                    return Err(Error::new(
                        ErrorKind::Conflict,
                        format!("a view named {new:?} already exists"),
                    ));
                }
            }
            if let Some(slot) = at.and_then(|i| schema.views.get_mut(i)) {
                *slot = def;
            } else if schema.views.len() >= MAX_VIEWS {
                return Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!("a collection may have at most {MAX_VIEWS} views"),
                ));
            } else {
                schema.views.push(def);
            }
            Ok(())
        })
        .await
    }

    /// Deletes a saved view.
    ///
    /// # Errors
    ///
    /// `not_found` when there is no view with that name.
    pub async fn delete_view(
        &self,
        ctx: &RequestContext,
        key: ViewKey,
    ) -> Result<CollectionInfo, Error> {
        let name = key.view;
        self.edit_schema(ctx, "delete_view", key.collection_id, move |schema| {
            let before = schema.views.len();
            schema.views.retain(|v| !same_name(&v.name, &name));
            if schema.views.len() == before {
                return Err(Error::new(
                    ErrorKind::NotFound,
                    format!("no view named {name:?}"),
                ));
            }
            Ok(())
        })
        .await
    }
}

impl CollectionsService {
    fn view_operations(this: &Arc<Self>) -> Vec<Operation> {
        let s = Arc::clone(this);
        let save_view = Operation::mutation(
            "collections.save_view",
            HttpMethod::Put,
            "/v1/collections/{collection_id}/views/{view}",
            Scope::WriteDirect,
            move |ctx, input: SaveViewInput| {
                let s = Arc::clone(&s);
                async move { s.save_view(&ctx, input).await }
            },
        )
        .summary("Save a view: its layout, filters, sorts, grouping and hidden columns");
        let s = Arc::clone(this);
        let delete_view = Operation::mutation(
            "collections.delete_view",
            HttpMethod::Delete,
            "/v1/collections/{collection_id}/views/{view}",
            Scope::WriteDirect,
            move |ctx, key: ViewKey| {
                let s = Arc::clone(&s);
                async move { s.delete_view(&ctx, key).await }
            },
        )
        .summary("Delete a saved view");
        vec![save_view, delete_view]
    }
}

fn embed_error(e: &crate::fence::EmbedError) -> Error {
    Error::new(ErrorKind::InvalidInput, e.0.clone())
}

impl CollectionsService {
    /// Runs the view a `collection` block in a page asks for, as
    /// `principal` sees it. The renderer and exporters call this directly;
    /// the web app calls `collections.embed`.
    ///
    /// # Errors
    ///
    /// `invalid_input` with a readable message for a malformed block or
    /// an unknown view or property; `not_found` when the collection does
    /// not exist or the caller cannot read it.
    pub async fn resolve_embed(
        &self,
        principal: &Principal,
        fence: &str,
    ) -> Result<EmbedResult, Error> {
        let embed = crate::fence::CollectionEmbed::parse(fence).map_err(|e| embed_error(&e))?;
        let state = self.read().await?;
        self.require_read(&state, principal, embed.id).await?;
        let (title, path) = match (state.ws.collection(embed.id), state.folder(embed.id)) {
            (Some(c), Ok(f)) => (c.schema().title.clone(), f.path.as_str().to_owned()),
            _ => {
                return Err(Error::new(
                    ErrorKind::NotFound,
                    format!("collection {} not found", embed.id),
                ));
            }
        };
        let mut computer = Computer::new(&state.ws, (self.clock)());
        let result = embed.resolve(&mut computer);
        drop(computer);
        drop(state);
        Ok(EmbedResult {
            collection_id: embed.id,
            title,
            path,
            result: result.map_err(|e| embed_error(&e))?,
        })
    }

    /// Computes a collection chart's data, as `principal` sees it.
    ///
    /// # Errors
    ///
    /// `invalid_input` for a source that is not `collection:<id>`, a
    /// missing `x`, an unknown property, view or aggregation; `not_found`
    /// when the collection cannot be read.
    pub async fn resolve_chart(
        &self,
        principal: &Principal,
        input: ChartInput,
    ) -> Result<crate::fence::ChartData, Error> {
        let mut attrs = std::collections::BTreeMap::from([
            ("source".to_owned(), input.source),
            ("x".to_owned(), input.x),
        ]);
        for (k, v) in [
            ("view", input.view),
            ("y", input.y),
            ("series", input.series),
            ("agg", input.agg),
        ] {
            if let Some(v) = v {
                let _ = attrs.insert(k.to_owned(), v);
            }
        }
        let source = crate::fence::ChartSource::from_attrs(&attrs)
            .map_err(|e| embed_error(&e))?
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidInput,
                    "a chart source must be collection:<id>",
                )
            })?;
        let state = self.read().await?;
        self.require_read(&state, principal, source.id).await?;
        let mut computer = Computer::new(&state.ws, (self.clock)());
        let data = source.resolve(&mut computer);
        drop(computer);
        drop(state);
        data.map_err(|e| embed_error(&e))
    }

    fn embed_operations(this: &Arc<Self>) -> Vec<Operation> {
        let s = Arc::clone(this);
        let embed = Operation::query(
            "collections.embed",
            "/v1/collections/embed",
            Scope::Read,
            move |ctx, input: EmbedInput| {
                let s = Arc::clone(&s);
                async move { s.resolve_embed(&ctx.principal, &input.fence).await }
            },
        )
        .summary("Show the live rows of a collection block in a page");
        let s = Arc::clone(this);
        let chart = Operation::query(
            "collections.chart",
            "/v1/collections/chart",
            Scope::Read,
            move |ctx, input: ChartInput| {
                let s = Arc::clone(&s);
                async move { s.resolve_chart(&ctx.principal, input).await }
            },
        )
        .summary("Compute a chart's data from a collection");
        vec![embed, chart]
    }
}

impl CollectionsService {
    /// Exports a view as CSV: its columns, filters and order, every row.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown collection or view.
    pub async fn export_csv(
        &self,
        ctx: &RequestContext,
        input: ExportCsvInput,
    ) -> Result<CsvExport, Error> {
        let state = self.read().await?;
        let id = input.collection_id;
        self.require_read(&state, &ctx.principal, id).await?;
        let schema = state
            .ws
            .collection(id)
            .map(crate::workspace::Collection::schema)
            .ok_or_else(|| Error::new(ErrorKind::NotFound, format!("collection {id} not found")))?;
        let def = view_def(schema, input.view.as_deref())?;
        let stem = if input.view.is_some() {
            format!("{} - {}", schema.title, def.name)
        } else {
            schema.title.clone()
        };
        let file_name = vellum_store::layout::file_name(&stem, "csv");
        let mut computer = Computer::new(&state.ws, (self.clock)());
        let csv = crate::csv::export(&mut computer, id, &def);
        drop(computer);
        drop(state);
        Ok(CsvExport {
            file_name,
            csv: csv?,
        })
    }

    /// Adds the rows of a CSV file to a collection, all or nothing.
    ///
    /// # Errors
    ///
    /// `invalid_input` for an unreadable or oversized file;
    /// `permission_denied` without edit access.
    pub async fn import_csv(
        &self,
        ctx: &RequestContext,
        input: ImportCsvInput,
    ) -> Result<CsvImported, Error> {
        csv_size(&input.csv)?;
        let id = input.collection_id;
        let span = tracing::info_span!("collections.import_csv", collection = %id);
        async move {
            // Match and apply the file under the read lock, so a large
            // import does not stall every other request.
            let (seen, prepared) = {
                let state = self.read().await?;
                let seen = self.writes.load(Ordering::Acquire);
                self.require_edit(&state, &ctx.principal, id, true).await?;
                let prepared = prepare_import(&state, &input.csv, id, input.dry_run)?;
                if input.dry_run {
                    let collection = collection_info(&state, id)?;
                    drop(state);
                    return Ok(prepared.report(false, collection));
                }
                drop(state);
                (seen, prepared)
            };
            let mut guard = self.state.write().await;
            let current = self.writes.load(Ordering::Acquire) == seen && guard.is_some();
            self.bump();
            if guard.is_none() {
                *guard = Some(State::load(self.store.as_ref()).await?);
            }
            let Some(state) = guard.as_mut() else {
                return Err(Error::new(
                    ErrorKind::Internal,
                    "collections were not loaded",
                ));
            };
            // Something changed in between: match again against what is
            // loaded now, rather than overwrite it with a stale copy.
            let mut prepared = if current {
                prepared
            } else {
                self.require_edit(state, &ctx.principal, id, true).await?;
                prepare_import(state, &input.csv, id, false)?
            };
            let changed = std::mem::take(&mut prepared.changed);
            if let Some(ws) = prepared.ws.take() {
                state.ws = ws;
            }
            let result = state.write(self.store.as_ref(), &changed).await;
            crate::telemetry::wrote("import_csv", result.is_ok());
            if let Err(e) = result {
                *guard = None;
                return Err(e);
            }
            self.feed(index::affected_rows(state, &changed));
            let collection = collection_info(state, id)?;
            drop(guard);
            prepared.changed = changed;
            Ok(prepared.report(true, collection))
        }
        .instrument(span)
        .await
    }

    /// Creates a collection from a CSV file, inferring each column's type.
    ///
    /// # Errors
    ///
    /// `permission_denied` without edit access to the folder; `conflict`
    /// when a collection with that name is already there.
    pub async fn create_from_csv(
        &self,
        ctx: &RequestContext,
        input: CreateFromCsvInput,
    ) -> Result<CsvImported, Error> {
        csv_size(&input.csv)?;
        let title = input.title.trim();
        if title.is_empty() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "a collection needs a title",
            ));
        }
        let folder = vellum_store::BlobPath::new(input.folder.trim_end_matches('/'))
            .map_err(state::store_error)?;
        if !matches!(
            vellum_store::layout::Kind::of(&folder.join("x.md").map_err(state::store_error)?),
            vellum_store::layout::Kind::Document
        ) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "collections can only be created in a drive folder",
            ));
        }
        if !self
            .access
            .allowed_in(&ctx.principal, &folder, Need::Edit)
            .await?
        {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "you cannot create collections in this folder",
            ));
        }
        let dir = folder
            .join(&vellum_store::layout::file_name(title, "collection"))
            .map_err(state::store_error)?;
        let id = CollectionId::generate();
        let import = crate::csv::import_new(&input.csv, id, title)?;
        let rows = import.rows.len();
        let (skipped_columns, diagnostics) =
            (import.skipped_columns.clone(), import.diagnostics.clone());
        let collection = import.into_collection();
        let info = CollectionInfo::new(collection.schema(), dir.as_str().to_owned(), Vec::new());
        let changed = crate::workspace::Changeset {
            written: collection
                .rows()
                .map(|r| crate::workspace::RowRef {
                    collection: id,
                    row: r.id,
                })
                .collect(),
            deleted: std::collections::BTreeSet::new(),
        };
        if !input.dry_run {
            let span = tracing::info_span!("collections.create_from_csv", collection = %id);
            async {
                let mut guard = self.state.write().await;
                self.bump();
                if guard.is_none() {
                    *guard = Some(State::load(self.store.as_ref()).await?);
                }
                let Some(state) = guard.as_mut() else {
                    return Err(Error::new(
                        ErrorKind::Internal,
                        "collections were not loaded",
                    ));
                };
                let result = state.create(self.store.as_ref(), dir, collection).await;
                crate::telemetry::wrote("create_from_csv", result.is_ok());
                if result.is_err() {
                    *guard = None;
                } else {
                    self.feed(changed.written.iter().map(|r| r.row).collect());
                }
                drop(guard);
                result
            }
            .instrument(span)
            .await?;
        }
        Ok(CsvImported {
            written: !input.dry_run,
            rows,
            skipped_columns,
            diagnostics,
            collection: info,
            changed,
        })
    }

    fn csv_operations(this: &Arc<Self>) -> Vec<Operation> {
        let s = Arc::clone(this);
        let export = Operation::query(
            "collections.export_csv",
            "/v1/collections/{collection_id}/csv",
            Scope::Read,
            move |ctx, input: ExportCsvInput| {
                let s = Arc::clone(&s);
                async move { s.export_csv(&ctx, input).await }
            },
        )
        .summary("Export a view as CSV");
        let s = Arc::clone(this);
        let import = Operation::mutation(
            "collections.import_csv",
            HttpMethod::Post,
            "/v1/collections/{collection_id}/csv",
            Scope::WriteDirect,
            move |ctx, input: ImportCsvInput| {
                let s = Arc::clone(&s);
                async move { s.import_csv(&ctx, input).await }
            },
        )
        .summary("Add the rows of a CSV file to a collection");
        let s = Arc::clone(this);
        let create = Operation::mutation(
            "collections.create_from_csv",
            HttpMethod::Post,
            "/v1/collections",
            Scope::WriteDirect,
            move |ctx, input: CreateFromCsvInput| {
                let s = Arc::clone(&s);
                async move { s.create_from_csv(&ctx, input).await }
            },
        )
        .summary("Create a collection from a CSV file, inferring column types");
        vec![export, import, create]
    }
}

impl Service for CollectionsService {
    fn name(&self) -> &'static str {
        "collections"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let s = Arc::clone(&self);
        let list = Operation::query(
            "collections.list",
            "/v1/collections",
            Scope::Read,
            move |ctx, _: ListCollections| {
                let s = Arc::clone(&s);
                async move { s.list(&ctx).await }
            },
        )
        .summary("List the collections you can read");
        let s = Arc::clone(&self);
        let get = Operation::query(
            "collections.get",
            "/v1/collections/{collection_id}",
            Scope::Read,
            move |ctx, key: CollectionKey| {
                let s = Arc::clone(&s);
                async move { s.get(&ctx, key.collection_id).await }
            },
        )
        .summary("Get a collection's properties and saved views");
        let s = Arc::clone(&self);
        let query = Operation::query(
            "collections.query",
            "/v1/collections/{collection_id}/query",
            Scope::Read,
            move |ctx, input: QueryInput| {
                let s = Arc::clone(&s);
                async move { s.query(&ctx, input).await }
            },
        )
        .summary("Run a view: filter, sort, group and compute rows");
        let s = Arc::clone(&self);
        let get_row = Operation::query(
            "collections.get_row",
            "/v1/collections/{collection_id}/rows/{row_id}",
            Scope::Read,
            move |ctx, key: RowKey| {
                let s = Arc::clone(&s);
                async move { s.get_row(&ctx, key).await }
            },
        )
        .summary("Get one row with its computed values");
        let s = Arc::clone(&self);
        let create_row = Operation::mutation(
            "collections.create_row",
            HttpMethod::Post,
            "/v1/collections/{collection_id}/rows",
            Scope::WriteDirect,
            move |ctx, input: CreateRowInput| {
                let s = Arc::clone(&s);
                async move { s.create_row(&ctx, input).await }
            },
        )
        .summary("Add a row");
        let s = Arc::clone(&self);
        let update_row = Operation::mutation(
            "collections.update_row",
            HttpMethod::Patch,
            "/v1/collections/{collection_id}/rows/{row_id}",
            Scope::WriteDirect,
            move |ctx, input: UpdateRowInput| {
                let s = Arc::clone(&s);
                async move { s.update_row(&ctx, input).await }
            },
        )
        .summary("Change a row's values or body");
        let views = Self::view_operations(&self);
        let embeds = Self::embed_operations(&self);
        let csv = Self::csv_operations(&self);
        let s = self;
        let delete_row = Operation::mutation(
            "collections.delete_row",
            HttpMethod::Delete,
            "/v1/collections/{collection_id}/rows/{row_id}",
            Scope::WriteDirect,
            move |ctx, key: RowKey| {
                let s = Arc::clone(&s);
                async move { s.delete_row(&ctx, key).await }
            },
        )
        .summary("Delete a row and unlink it");
        let mut ops = vec![
            list, get, query, get_row, create_row, update_row, delete_row,
        ];
        ops.extend(views);
        ops.extend(embeds);
        ops.extend(csv);
        ops
    }

    fn readiness(&self) -> BoxFuture<'_, Readiness> {
        Box::pin(async move {
            match self.read().await {
                Ok(_) => Readiness::Ready,
                Err(e) => Readiness::NotReady(format!("collections could not load: {e}")),
            }
        })
    }
}
