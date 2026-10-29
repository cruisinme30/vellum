// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! [`BooksService`]: the operations, their access rules and the
//! [`Service`] registration.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{Mutex, RwLock};
use tracing::Instrument as _;
use vellum_authz::{Permission, Relation};
use vellum_books::manifest::{NumberingStyle, Principal as BookPrincipal};
use vellum_books::ops::{NodeRef, Place};
use vellum_books::snapshot::{self, BookDiff};
use vellum_books::summary::Section;
use vellum_books::{Book, NodeId, markdown};
use vellum_contracts::{
    BoxFuture, HttpMethod, Operation, Principal, Readiness, RequestContext, Scope, Service,
};
use vellum_kernel::{BookId, Error, ErrorKind, Timestamp};
use vellum_store::{BlobPath, BlobStore};

use crate::access::Access;
use crate::api::{
    AddSection, At, BookInfo, BookKey, BookList, BookSummary, CreateSnapshot, DiffChanged,
    DiffInput, DiffMoved, DiffOutput, DiffSection, ListBooks, MoveSection, PlaceInput, Region,
    Removed, SectionAccessInfo, SectionDoc, SectionKey, SetSectionAccess, SnapshotInfo,
    SnapshotList, TocLine, TocNode, UpdateSection,
};
use crate::files::{self, Loaded, book_error};
use crate::mirror::Restrictions;

/// Serves books over the gateway.
pub struct BooksService {
    store: Arc<dyn BlobStore>,
    access: Arc<dyn Access>,
    books: RwLock<Option<BTreeMap<BookId, BlobPath>>>,
    writes: Mutex<()>,
}

impl fmt::Debug for BooksService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BooksService")
            .field("store", &self.store)
            .finish_non_exhaustive()
    }
}

/// A book opened for one request.
struct Opened {
    id: BookId,
    relation: Relation,
    principals: BTreeSet<BookPrincipal>,
    loaded: Loaded,
}

fn not_found(what: &str) -> Error {
    Error::new(ErrorKind::NotFound, format!("{what} not found"))
}

impl Opened {
    const fn book(&self) -> &Book {
        &self.loaded.book
    }

    /// Book managers and owners see every section, as in the
    /// authorization engine's narrowing rule.
    fn sees_all(&self) -> bool {
        self.relation.implies(Relation::Manager)
    }

    fn can_see(&self, id: &NodeId) -> Result<bool, Error> {
        Ok(self.sees_all()
            || self
                .book()
                .can_view(id, &self.principals)
                .map_err(book_error)?)
    }

    /// The sections the caller cannot see.
    fn hidden(&self) -> BTreeSet<NodeId> {
        if self.sees_all() {
            return BTreeSet::new();
        }
        hidden_in(self.book(), &self.principals)
    }

    /// The table of contents without hidden sections. Numbers are the
    /// book's own, so every reader quotes the same ones (D145).
    fn toc(&self) -> Vec<TocLine> {
        let hidden: BTreeSet<String> = self.hidden().iter().map(ToString::to_string).collect();
        self.book()
            .toc()
            .items
            .iter()
            .map(TocLine::from)
            .filter_map(|line| match line {
                TocLine::Chapter(node) => prune(node, &hidden).map(TocLine::Chapter),
                other => Some(other),
            })
            .collect()
    }

    /// A section the caller can see; anything else is "not found", so a
    /// caller cannot probe for hidden sections.
    fn section(&self, raw: &str) -> Result<NodeId, Error> {
        let id = NodeId::parse(raw).map_err(|_| not_found("section"))?;
        if self.book().contains(&id) && self.can_see(&id)? {
            Ok(id)
        } else {
            Err(not_found("section"))
        }
    }

    fn place(&self, input: &PlaceInput) -> Result<Place, Error> {
        let target = || -> Result<NodeRef, Error> {
            let raw = input
                .section
                .as_deref()
                .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "this place needs a section"))?;
            Ok(NodeRef::Id(self.section(raw)?))
        };
        let region = match input.region.unwrap_or(Region::Numbered) {
            Region::Prefix => Section::Prefix,
            Region::Numbered => Section::Numbered,
            Region::Suffix => Section::Suffix,
        };
        Ok(match input.at {
            At::Before => Place::Before(target()?),
            At::After => Place::After(target()?),
            At::FirstChildOf => Place::FirstChildOf(target()?),
            At::LastChildOf => Place::LastChildOf(target()?),
            At::Start => Place::Start(region),
            At::End => Place::End(region),
        })
    }

    fn access_info(&self, id: &NodeId) -> Result<SectionAccessInfo, Error> {
        let access = self.book().access(id).map_err(book_error)?;
        let list = |set: &BTreeSet<BookPrincipal>| set.iter().map(ToString::to_string).collect();
        let own = self
            .book()
            .manifest()
            .nodes
            .get(id)
            .and_then(|e| e.restrict_to.as_ref())
            .map(list);
        Ok(SectionAccessInfo {
            restrict_to: access.restrict_to.as_ref().map(list),
            restricted_by: access
                .restricted_by
                .iter()
                .map(ToString::to_string)
                .collect(),
            own_restrict_to: own,
            owners: list(&access.owners),
            reviewers: list(&access.reviewers),
        })
    }

    fn section_doc(&self, id: &NodeId) -> Result<SectionDoc, Error> {
        let book = self.book();
        let toc = book.toc();
        let entry = toc.find(id).ok_or_else(|| not_found("section"))?;
        let source = book.text(id).map_err(book_error)?.to_owned();
        let hidden = self.hidden();
        Ok(SectionDoc {
            book_id: self.id,
            id: id.to_string(),
            title: entry.title.clone(),
            label: entry.label.clone(),
            number: entry.number_label.clone(),
            path: book.path_of(id).map_err(book_error)?.as_str().to_owned(),
            markdown: book
                .render_cross_refs_for(markdown::body(&source), |target| !hidden.contains(target)),
            source,
            restricted: book.access(id).map_err(book_error)?.is_restricted(),
            access: if self.sees_all() {
                Some(self.access_info(id)?)
            } else {
                None
            },
        })
    }
}

/// The sections of `book` someone holding `principals` cannot see.
fn hidden_in(book: &Book, principals: &BTreeSet<BookPrincipal>) -> BTreeSet<NodeId> {
    book.manifest()
        .nodes
        .keys()
        .filter(|id| !book.can_view(id, principals).unwrap_or(false))
        .cloned()
        .collect()
}

/// `node` without the hidden sections under it, or `None` if it is hidden
/// itself. A restriction covers everything under it, so a hidden section
/// takes its children with it.
fn prune(mut node: TocNode, hidden: &BTreeSet<String>) -> Option<TocNode> {
    if node.id.as_ref().is_some_and(|id| hidden.contains(id)) {
        return None;
    }
    node.children = std::mem::take(&mut node.children)
        .into_iter()
        .filter_map(|child| prune(child, hidden))
        .collect();
    Some(node)
}

const fn numbering(style: NumberingStyle) -> &'static str {
    match style {
        NumberingStyle::Decimal => "decimal",
        NumberingStyle::Outline => "outline",
        NumberingStyle::None => "none",
    }
}

fn principals(raw: &[String]) -> Result<BTreeSet<BookPrincipal>, Error> {
    raw.iter()
        .map(|p| BookPrincipal::parse(p).map_err(book_error))
        .collect()
}

fn observe<T>(
    op: &'static str,
    fut: impl Future<Output = Result<T, Error>>,
) -> impl Future<Output = Result<T, Error>> {
    let span = tracing::info_span!("books.service", books.op = op);
    async move {
        let started = Instant::now();
        let result = fut.await;
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        tracing::info!(
            histogram.vellum.books.request_ms = ms,
            books.op = op,
            books.ok = result.is_ok(),
            "books request"
        );
        result
    }
    .instrument(span)
}

impl BooksService {
    /// A service over `store`, checking every request with `access`.
    #[must_use]
    pub fn new(store: Arc<dyn BlobStore>, access: Arc<dyn Access>) -> Self {
        Self {
            store,
            access,
            books: RwLock::new(None),
            writes: Mutex::new(()),
        }
    }

    /// Forgets where books are; the next request lists the store again.
    /// The watcher calls this when a `book.yaml` appears, moves or goes.
    pub async fn invalidate(&self) {
        *self.books.write().await = None;
    }

    async fn dirs(&self) -> Result<BTreeMap<BookId, BlobPath>, Error> {
        if let Some(books) = self.books.read().await.as_ref() {
            return Ok(books.clone());
        }
        let found = files::locate(self.store.as_ref()).await?;
        *self.books.write().await = Some(found.clone());
        Ok(found)
    }

    async fn dir_of(&self, id: BookId) -> Result<BlobPath, Error> {
        if let Some(dir) = self.dirs().await?.remove(&id) {
            return Ok(dir);
        }
        // A book created or moved since the last listing.
        self.invalidate().await;
        self.dirs()
            .await?
            .remove(&id)
            .ok_or_else(|| not_found("book"))
    }

    /// Opens a book for `principal`, who needs `need` on it. A book they
    /// cannot read is "not found".
    async fn open(
        &self,
        principal: &Principal,
        id: BookId,
        need: Permission,
    ) -> Result<Opened, Error> {
        let dir = self.dir_of(id).await?;
        let relation = self
            .access
            .relation(principal, id, &dir)
            .await?
            .ok_or_else(|| not_found("book"))?;
        if !relation.implies(need.required()) {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                format!("you need to be able to {} this book", need.as_str()),
            ));
        }
        let principals = self.access.principals(principal).await?;
        let loaded = files::load(self.store.as_ref(), &dir).await?;
        if loaded.book.id().as_str() != id.to_string() {
            self.invalidate().await;
            return Err(not_found("book"));
        }
        Ok(Opened {
            id,
            relation,
            principals,
            loaded,
        })
    }

    /// Opens, edits, saves and mirrors one book. Edits run one at a time.
    async fn edit<T>(
        &self,
        principal: &Principal,
        id: BookId,
        need: Permission,
        change: impl FnOnce(&mut Opened) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let _one_at_a_time = self.writes.lock().await;
        let mut opened = self.open(principal, id, need).await?;
        let out = change(&mut opened)?;
        let _ = opened.loaded.save(self.store.as_ref()).await?;
        self.mirror(&opened.loaded).await?;
        Ok(out)
    }

    async fn mirror(&self, loaded: &Loaded) -> Result<(), Error> {
        let id = loaded
            .book
            .id()
            .as_str()
            .parse::<BookId>()
            .map_err(|_| not_found("book"))?;
        let restrictions = Restrictions::of(&loaded.book).map_err(book_error)?;
        let report = self
            .access
            .mirror(id, &loaded.dir, &restrictions)
            .await
            .map_err(|e| {
                Error::new(
                    ErrorKind::Unavailable,
                    "the book was saved, but its section permissions could not be synced",
                )
                .with_source(e)
            })?;
        if report.skipped > 0 {
            tracing::debug!(
                book = %id,
                books.unindexed = report.skipped,
                "sections not indexed yet; the service enforces them meanwhile"
            );
        }
        Ok(())
    }

    /// Re-applies a book's section restrictions to the authorization
    /// engine. The watcher calls this after `book.yaml` changes on disk,
    /// and the indexer after it registers new sections.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown book, or when the engine is down.
    pub async fn sync_access(&self, id: BookId) -> Result<(), Error> {
        observe("sync_access", async {
            let dir = self.dir_of(id).await?;
            let loaded = files::load(self.store.as_ref(), &dir).await?;
            self.mirror(&loaded).await
        })
        .await
    }

    /// The books the caller can open.
    ///
    /// # Errors
    ///
    /// When the store or the access check is unavailable.
    pub async fn list(&self, ctx: &RequestContext) -> Result<BookList, Error> {
        observe("list", async {
            let mut books = Vec::new();
            for (id, dir) in self.dirs().await? {
                if self
                    .access
                    .relation(&ctx.principal, id, &dir)
                    .await?
                    .is_none()
                {
                    continue;
                }
                let Ok(loaded) = files::load(self.store.as_ref(), &dir).await else {
                    continue;
                };
                books.push(BookSummary {
                    id,
                    title: loaded.book.title().to_owned(),
                    path: dir.as_str().to_owned(),
                });
            }
            books.sort_by(|a, b| {
                a.title
                    .to_lowercase()
                    .cmp(&b.title.to_lowercase())
                    .then_with(|| a.id.cmp(&b.id))
            });
            Ok(BookList { books })
        })
        .await
    }

    /// A book's settings and table of contents, as the caller sees it.
    ///
    /// # Errors
    ///
    /// `not_found` when it does not exist or the caller cannot open it.
    pub async fn get(&self, ctx: &RequestContext, id: BookId) -> Result<BookInfo, Error> {
        observe("get", async {
            let opened = self.open(&ctx.principal, id, Permission::Read).await?;
            let book = opened.book();
            let manifest = book.manifest();
            Ok(BookInfo {
                id,
                title: book.title().to_owned(),
                numbering: numbering(manifest.numbering).to_owned(),
                max_depth: manifest.max_depth,
                role: opened.relation.as_str().to_owned(),
                toc: opened.toc(),
            })
        })
        .await
    }

    /// One section, with cross-references filled in.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown or hidden section.
    pub async fn get_section(
        &self,
        ctx: &RequestContext,
        key: SectionKey,
    ) -> Result<SectionDoc, Error> {
        observe("get_section", async {
            let opened = self
                .open(&ctx.principal, key.book_id, Permission::Read)
                .await?;
            let id = opened.section(&key.section_id)?;
            opened.section_doc(&id)
        })
        .await
    }

    /// Adds a section.
    ///
    /// # Errors
    ///
    /// `permission_denied` without edit access, `invalid_input` for a
    /// place that does not exist or nests too deep.
    pub async fn add_section(
        &self,
        ctx: &RequestContext,
        input: AddSection,
    ) -> Result<SectionDoc, Error> {
        observe("add_section", async {
            let mut ids = files::fresh_ids();
            let id = self
                .edit(&ctx.principal, input.book_id, Permission::Edit, |o| {
                    let place = o.place(&input.place)?;
                    o.loaded
                        .book
                        .add_section(&place, &input.title, &input.body, &mut ids)
                        .map_err(book_error)
                })
                .await?;
            self.get_section(
                ctx,
                SectionKey {
                    book_id: input.book_id,
                    section_id: id.to_string(),
                },
            )
            .await
        })
        .await
    }

    /// Moves a section and everything under it.
    ///
    /// # Errors
    ///
    /// As for [`BooksService::add_section`]; also `invalid_input` for a
    /// move into the section itself.
    pub async fn move_section(
        &self,
        ctx: &RequestContext,
        input: MoveSection,
    ) -> Result<SectionDoc, Error> {
        observe("move_section", async {
            let id = self
                .edit(&ctx.principal, input.book_id, Permission::Edit, |o| {
                    let id = o.section(&input.section_id)?;
                    let place = o.place(&input.place)?;
                    o.loaded
                        .book
                        .move_section(&NodeRef::Id(id.clone()), &place)
                        .map_err(book_error)?;
                    Ok(id)
                })
                .await?;
            self.get_section(
                ctx,
                SectionKey {
                    book_id: input.book_id,
                    section_id: id.to_string(),
                },
            )
            .await
        })
        .await
    }

    /// Renames a section or replaces its body.
    ///
    /// # Errors
    ///
    /// As for [`BooksService::add_section`].
    pub async fn update_section(
        &self,
        ctx: &RequestContext,
        input: UpdateSection,
    ) -> Result<SectionDoc, Error> {
        observe("update_section", async {
            let id = self
                .edit(&ctx.principal, input.book_id, Permission::Edit, |o| {
                    let id = o.section(&input.section_id)?;
                    let book = &mut o.loaded.book;
                    if let Some(title) = &input.title {
                        book.rename(&NodeRef::Id(id.clone()), title)
                            .map_err(book_error)?;
                    }
                    if let Some(body) = &input.body {
                        let text = book.text(&id).map_err(book_error)?;
                        let start = markdown::split(text).body;
                        let mut updated = text.get(..start).unwrap_or_default().to_owned();
                        updated.push_str(body);
                        if !updated.ends_with('\n') {
                            updated.push('\n');
                        }
                        book.set_text(&id, updated).map_err(book_error)?;
                    }
                    Ok(id)
                })
                .await?;
            self.get_section(
                ctx,
                SectionKey {
                    book_id: input.book_id,
                    section_id: id.to_string(),
                },
            )
            .await
        })
        .await
    }

    /// Removes a section and everything under it. Their files are deleted;
    /// snapshots and the store's history keep earlier versions.
    ///
    /// # Errors
    ///
    /// `permission_denied` when the subtree holds sections the caller
    /// cannot see.
    pub async fn remove_section(
        &self,
        ctx: &RequestContext,
        key: SectionKey,
    ) -> Result<Removed, Error> {
        observe("remove_section", async {
            self.edit(&ctx.principal, key.book_id, Permission::Edit, |o| {
                let id = o.section(&key.section_id)?;
                for inner in o.book().subtree_ids(&id).map_err(book_error)? {
                    if !o.can_see(&inner)? {
                        return Err(Error::new(
                            ErrorKind::PermissionDenied,
                            "this section holds sections you cannot see",
                        ));
                    }
                }
                let removed = o
                    .loaded
                    .book
                    .remove_section(&NodeRef::Id(id))
                    .map_err(book_error)?;
                Ok(Removed {
                    removed: removed.iter().map(ToString::to_string).collect(),
                })
            })
            .await
        })
        .await
    }

    /// Replaces a section's own restriction, owners and reviewers, and
    /// mirrors the result into the authorization engine.
    ///
    /// # Errors
    ///
    /// `permission_denied` without manage access to the book,
    /// `invalid_input` for a malformed principal or a widening.
    pub async fn set_section_access(
        &self,
        ctx: &RequestContext,
        input: SetSectionAccess,
    ) -> Result<SectionAccessInfo, Error> {
        observe("set_section_access", async {
            self.edit(&ctx.principal, input.book_id, Permission::Manage, |o| {
                let id = o.section(&input.section_id)?;
                let restrict = principals(&input.restrict_to)?;
                let book = &mut o.loaded.book;
                book.set_restriction(&id, (!restrict.is_empty()).then_some(restrict))
                    .map_err(book_error)?;
                book.set_owners(&id, principals(&input.owners)?)
                    .map_err(book_error)?;
                book.set_reviewers(&id, principals(&input.reviewers)?)
                    .map_err(book_error)?;
                o.access_info(&id)
            })
            .await
        })
        .await
    }

    /// The book's snapshots.
    ///
    /// # Errors
    ///
    /// `not_found` when the caller cannot open the book.
    pub async fn list_snapshots(
        &self,
        ctx: &RequestContext,
        key: BookKey,
    ) -> Result<SnapshotList, Error> {
        observe("list_snapshots", async {
            let opened = self
                .open(&ctx.principal, key.book_id, Permission::Read)
                .await?;
            let mut snapshots = Vec::new();
            for tag in files::tags(self.store.as_ref(), &opened.loaded.dir).await? {
                let s = files::read_snapshot(self.store.as_ref(), &opened.loaded.dir, &tag).await?;
                snapshots.push(SnapshotInfo {
                    tag: s.tag,
                    message: s.message,
                    created_at: s.created_at,
                    created_by: s.created_by,
                });
            }
            Ok(SnapshotList { snapshots })
        })
        .await
    }

    /// Tags the current book.
    ///
    /// # Errors
    ///
    /// `conflict` for a tag that exists, `invalid_input` for a malformed
    /// one, `permission_denied` without edit access.
    pub async fn create_snapshot(
        &self,
        ctx: &RequestContext,
        input: CreateSnapshot,
    ) -> Result<SnapshotInfo, Error> {
        observe("create_snapshot", async {
            let opened = self
                .open(&ctx.principal, input.book_id, Permission::Edit)
                .await?;
            let dir = &opened.loaded.dir;
            let existing = files::tags(self.store.as_ref(), dir).await?;
            let by = ctx
                .principal
                .user_id()
                .map_or_else(String::new, |u| format!("user:{u}"));
            let snapshot = opened
                .book()
                .snapshot(
                    &input.tag,
                    &input.message,
                    &Timestamp::now().to_string(),
                    &by,
                    &existing,
                )
                .map_err(|e| {
                    let mut error = book_error(e);
                    if existing.contains(&input.tag) {
                        error = Error::new(ErrorKind::Conflict, error.to_string());
                    }
                    error
                })?;
            files::write_snapshot(self.store.as_ref(), dir, &snapshot).await?;
            Ok(SnapshotInfo {
                tag: snapshot.tag,
                message: snapshot.message,
                created_at: snapshot.created_at,
                created_by: snapshot.created_by,
            })
        })
        .await
    }

    /// What changed between a snapshot and another snapshot or now.
    ///
    /// # Errors
    ///
    /// `not_found` for an unknown tag.
    pub async fn diff(&self, ctx: &RequestContext, input: DiffInput) -> Result<DiffOutput, Error> {
        observe("diff", async {
            let opened = self
                .open(&ctx.principal, input.book_id, Permission::Read)
                .await?;
            let dir = &opened.loaded.dir;
            let old = files::read_snapshot(self.store.as_ref(), dir, &input.from)
                .await?
                .book()
                .map_err(book_error)?;
            let new = match &input.to {
                Some(tag) => files::read_snapshot(self.store.as_ref(), dir, tag)
                    .await?
                    .book()
                    .map_err(book_error)?,
                None => opened.book().clone(),
            };
            if opened.sees_all() {
                return Ok(output(snapshot::diff(&old, &new), |_| true, true));
            }
            // A section the caller cannot see now stays hidden even where
            // an older version showed it; one that no longer exists is
            // judged by the version that has it. Labels keep the book's
            // own numbering.
            let p = &opened.principals;
            let current = opened.book();
            let mut hidden = hidden_in(current, p);
            for side in [&old, &new] {
                hidden.extend(
                    hidden_in(side, p)
                        .into_iter()
                        .filter(|id| !current.contains(id)),
                );
            }
            let hidden: BTreeSet<String> = hidden.iter().map(ToString::to_string).collect();
            let changes = snapshot::diff(&old, &new);
            Ok(output(changes, |id| !hidden.contains(id), false))
        })
        .await
    }
}

fn output(diff: BookDiff, keep: impl Fn(&str) -> bool, full: bool) -> DiffOutput {
    DiffOutput {
        added: diff
            .added
            .into_iter()
            .map(|s| DiffSection {
                id: s.id.to_string(),
                label: s.label,
            })
            .filter(|s| keep(&s.id))
            .collect(),
        removed: diff
            .removed
            .into_iter()
            .map(|s| DiffSection {
                id: s.id.to_string(),
                label: s.label,
            })
            .filter(|s| keep(&s.id))
            .collect(),
        moved: diff
            .moved
            .into_iter()
            .map(|m| DiffMoved {
                id: m.id.to_string(),
                from: m.from,
                to: m.to,
            })
            .filter(|m| keep(&m.id))
            .collect(),
        changed: diff
            .changed
            .into_iter()
            .map(|c| DiffChanged {
                id: c.id.to_string(),
                label: c.label,
                added: c.added,
                removed: c.removed,
                patch: c.patch,
            })
            .filter(|c| keep(&c.id))
            .collect(),
        settings: if full { diff.settings } else { Vec::new() },
        summary_patch: if full {
            diff.summary_patch
        } else {
            String::new()
        },
    }
}

macro_rules! op {
    ($service:expr, query $id:literal, $path:literal, $scope:expr, $method:ident, $summary:literal) => {{
        let s = Arc::clone(&$service);
        Operation::query($id, $path, $scope, move |ctx, input| {
            let s = Arc::clone(&s);
            async move { s.$method(&ctx, input).await }
        })
        .summary($summary)
    }};
    ($service:expr, $verb:ident $id:literal, $path:literal, $scope:expr, $method:ident, $summary:literal) => {{
        let s = Arc::clone(&$service);
        Operation::mutation($id, HttpMethod::$verb, $path, $scope, move |ctx, input| {
            let s = Arc::clone(&s);
            async move { s.$method(&ctx, input).await }
        })
        .summary($summary)
    }};
}

impl BooksService {
    async fn get_op(&self, ctx: &RequestContext, key: BookKey) -> Result<BookInfo, Error> {
        self.get(ctx, key.book_id).await
    }

    async fn list_op(&self, ctx: &RequestContext, _: ListBooks) -> Result<BookList, Error> {
        self.list(ctx).await
    }
}

impl Service for BooksService {
    fn name(&self) -> &'static str {
        "books"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        vec![
            op!(self, query "books.list", "/v1/books", Scope::Read, list_op,
                "List the books you can open"),
            op!(self, query "books.get", "/v1/books/{book_id}", Scope::Read, get_op,
                "Get a book's settings and table of contents"),
            op!(self, query "books.get_section", "/v1/books/{book_id}/sections/{section_id}",
                Scope::Read, get_section, "Get one section with cross-references filled in"),
            op!(self, Post "books.add_section", "/v1/books/{book_id}/sections",
                Scope::WriteDirect, add_section, "Add a section"),
            op!(self, Post "books.move_section", "/v1/books/{book_id}/sections/{section_id}/move",
                Scope::WriteDirect, move_section, "Move or nest a section with everything under it"),
            op!(self, Patch "books.update_section", "/v1/books/{book_id}/sections/{section_id}",
                Scope::WriteDirect, update_section, "Rename a section or replace its body"),
            op!(self, Delete "books.remove_section", "/v1/books/{book_id}/sections/{section_id}",
                Scope::WriteDirect, remove_section, "Remove a section with everything under it"),
            op!(self, Put "books.set_section_access",
                "/v1/books/{book_id}/sections/{section_id}/access",
                Scope::WriteDirect, set_section_access,
                "Restrict a section and set its owners and reviewers"),
            op!(self, query "books.list_snapshots", "/v1/books/{book_id}/snapshots", Scope::Read,
                list_snapshots, "List a book's tagged snapshots"),
            op!(self, Post "books.create_snapshot", "/v1/books/{book_id}/snapshots",
                Scope::WriteDirect, create_snapshot, "Tag the current book"),
            op!(self, query "books.diff", "/v1/books/{book_id}/diff", Scope::Read, diff,
                "Compare a snapshot with another snapshot or the current book"),
        ]
    }

    /// Books are listed from the files, so a failure here is the blob
    /// store's: degraded, not unready (ADR 0044, D188). The storage
    /// service decides whether the store being down takes the replica out.
    fn readiness(&self) -> BoxFuture<'_, Readiness> {
        Box::pin(async move {
            match self.dirs().await {
                Ok(_) => Readiness::Ready,
                Err(e) => Readiness::Degraded(format!("books could not be listed: {e}")),
            }
        })
    }
}
