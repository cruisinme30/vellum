// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The MCP server's view of the real workspace (ADR 0019, D217–D218).
//!
//! Every tool runs as the signed-in user, through the same services the
//! web app and the REST API use, so search filtering, permissions,
//! validation and the activity log are the ones those enforce. Pages are
//! read and written through the drive as a model acting for the user;
//! issues, comments, tasks, collections and books go through their
//! services' own operations. The MCP layer has already checked the
//! token's scopes for the tool before any of this runs.
//!
//! Model writes in draft mode never change an existing page: a new page is
//! created with `status: draft`, and an edit, or a set of rows, becomes a
//! draft page in the user's My Drive that names what it proposes to
//! change (`draft_of`) and the version it was based on.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::{Map, Value, json};
use vellum_contracts::{
    CredentialKind, Operation, RequestContext, ScopeSet, Service,
};
use vellum_drive::{Drive, DriveIndex, Filter, NodeKind, Saved, Space};
use vellum_kernel::{
    Actor as Who, Error, ErrorKind, ObjectId, RequestId, SpaceId, Timestamp,
    UserId,
};
use vellum_mcp::auth::Principal;
use vellum_mcp::ports::{
    Actor, Attribution, Block, BlockSelector, Change, ChangePage,
    CollectionQuery, Document, EditProposal, ExportArtifact, ExportRequest,
    Issue, IssueUpdate, Listing, ListingPage, NewComment, NewIssue, NewPage,
    NewTask, OutlineNode, RowPage, RowUpsert, SearchHit, SearchQuery, SiteInfo,
    Template, Workspace, WorkspaceError, WriteMode, WriteReceipt,
};
use vellum_private::PrivateDoc;
use vellum_store::Base;

use crate::private::{blocks_of, select};

/// The operations the adapter calls; nothing else is reachable through it.
const OPS: [&str; 14] = [
    "search.query",
    "work.get_issue",
    "work.create_issue",
    "work.update_issue",
    "work.add_comment",
    "tasks.create",
    "tasks.update",
    "collections.list",
    "collections.query",
    "collections.create_row",
    "collections.update_row",
    "books.list",
    "books.get",
    "collections.get",
];

/// The folder name that holds page templates.
const TEMPLATES: &str = "Templates";

/// Most template folders, and templates per folder, looked at.
const MAX_TEMPLATES: usize = 200;

/// A template's name: its title in lowercase, words joined by hyphens.
fn template_name(title: &str) -> String {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

/// How long one service call may take.
const BUDGET: Duration = Duration::from_secs(30);

/// Most pages listed in `llms.txt` and the sitemap.
const LISTED_PAGES: usize = 5_000;

/// Most activity entries read per space for `list_changes`.
const ACTIVITY_PER_SPACE: usize = 500;

/// The signed-in caller of one tool call.
struct Caller {
    user: UserId,
    principal: vellum_contracts::Principal,
}

impl Caller {
    fn of(actor: &Actor) -> Result<Self, WorkspaceError> {
        let Actor::User(p) = actor else {
            return Err(WorkspaceError::NotFound(
                "sign in to read the workspace".into(),
            ));
        };
        Self::from_principal(p)
    }

    fn from_principal(p: &Principal) -> Result<Self, WorkspaceError> {
        let user: UserId = p
            .user_id
            .parse()
            .map_err(|_| WorkspaceError::NotFound("unknown user".into()))?;
        let scopes = ScopeSet::from_scopes(
            p.scopes.iter().filter_map(|s| s.as_str().parse().ok()),
        );
        Ok(Self {
            user,
            principal: vellum_contracts::Principal::user(
                user,
                CredentialKind::ApiToken,
                scopes,
                Timestamp::now(),
            ),
        })
    }

    /// The drive actor: a model acting for the user.
    const fn model(&self) -> Who {
        Who::Model {
            on_behalf_of: self.user,
        }
    }
}

/// A failed read: what the caller may not see does not exist.
fn read_err(what: &str) -> impl Fn(Error) -> WorkspaceError + '_ {
    move |e| match e.kind() {
        ErrorKind::NotFound
        | ErrorKind::PermissionDenied
        | ErrorKind::Unauthenticated => {
            WorkspaceError::NotFound(what.to_owned())
        }
        _ => write_error(&e),
    }
}

/// A failed write.
fn write_error(e: &Error) -> WorkspaceError {
    let message = e.message().to_owned();
    match e.kind() {
        ErrorKind::NotFound => WorkspaceError::NotFound(message),
        ErrorKind::PermissionDenied | ErrorKind::Unauthenticated => {
            WorkspaceError::Forbidden(message)
        }
        ErrorKind::InvalidInput => WorkspaceError::Invalid(message),
        ErrorKind::Conflict => WorkspaceError::Conflict(message),
        _ => WorkspaceError::Unavailable(message),
    }
}

fn write_err(e: impl Into<Error>) -> WorkspaceError {
    write_error(&e.into())
}

fn object_id(id: &str) -> Result<ObjectId, WorkspaceError> {
    id.trim()
        .parse()
        .map_err(|_| WorkspaceError::NotFound(id.to_owned()))
}

fn text(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Offset paging: the cursor is the index of the next item.
fn page<T>(
    items: Vec<T>,
    cursor: Option<&str>,
    limit: u32,
) -> Result<(Vec<T>, Option<String>), WorkspaceError> {
    let start = cursor.map_or(Ok(0), |c| {
        c.parse::<usize>()
            .map_err(|_| WorkspaceError::Invalid("cursor".into()))
    })?;
    let limit = usize::try_from(limit).unwrap_or(usize::MAX).max(1);
    let total = items.len();
    let end = start.saturating_add(limit).min(total);
    let items: Vec<T> = items
        .into_iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect();
    Ok((items, (end < total).then(|| end.to_string())))
}

/// A date or time as the start of a window: RFC 3339, or `YYYY-MM-DD` for
/// midnight UTC.
fn since(text: &str) -> Result<Timestamp, WorkspaceError> {
    let text = text.trim();
    let full = if text.len() == 10 {
        format!("{text}T00:00:00Z")
    } else {
        text.to_owned()
    };
    full.parse()
        .map_err(|_| WorkspaceError::Invalid(format!("{text} is not a date")))
}

/// The front matter recorded on every page a model writes.
fn attribution(by: &Attribution) -> Vec<(&'static str, String)> {
    vec![
        ("authored_by", by.authored_by.clone()),
        ("on_behalf_of", by.on_behalf_of.clone()),
        ("client", by.client.clone()),
    ]
}

/// A page's body as Vellum Markdown, without its front matter.
fn body_of(markdown: &str) -> String {
    let mut doc = vellum_format::parse(markdown).document;
    doc.front_matter = None;
    vellum_format::serialize(&doc)
}

/// Builds the outline tree from headings in reading order.
fn nest(flat: Vec<OutlineNode>) -> Vec<OutlineNode> {
    fn take(
        flat: &mut std::iter::Peekable<std::vec::IntoIter<OutlineNode>>,
        depth: u8,
    ) -> Vec<OutlineNode> {
        let mut out = Vec::new();
        while let Some(next) = flat.peek() {
            if next.depth < depth {
                break;
            }
            let Some(mut node) = flat.next() else { break };
            node.children = take(flat, node.depth.saturating_add(1));
            out.push(node);
        }
        out
    }
    let min = flat.iter().map(|n| n.depth).min().unwrap_or(1);
    take(&mut flat.into_iter().peekable(), min)
}

/// A collection filter object (`{property, op, value}`) as filter text.
fn filter_text(filter: &Value) -> Result<String, WorkspaceError> {
    let property = text(filter, "property");
    if property.is_empty() {
        return Err(WorkspaceError::Invalid(
            "a filter needs a property".into(),
        ));
    }
    let op = match text(filter, "op").as_str() {
        "" | "eq" | "is" | "=" => "=",
        "neq" | "is_not" | "!=" => "!=",
        "contains" => "contains",
        "not_contains" => "not contains",
        "lt" | "<" => "<",
        "lte" | "<=" => "<=",
        "gt" | ">" => ">",
        "gte" | ">=" => ">=",
        "empty" | "is_empty" => return Ok(format!("\"{property}\" is empty")),
        "not_empty" | "is_not_empty" => {
            return Ok(format!("\"{property}\" is not empty"));
        }
        other => {
            return Err(WorkspaceError::Invalid(format!(
                "unknown filter op {other}"
            )));
        }
    };
    let value = match filter.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    };
    Ok(format!("\"{property}\" {op} {value}"))
}

/// Sets `key` on a JSON object.
fn put(input: &mut Value, key: &str, value: Value) {
    if let Some(object) = input.as_object_mut() {
        let _ = object.insert(key.to_owned(), value);
    }
}

fn sort_text(sort: &Value) -> String {
    let direction = if text(sort, "direction").eq_ignore_ascii_case("desc") {
        " desc"
    } else {
        ""
    };
    format!("\"{}\"{direction}", text(sort, "property"))
}

/// The workspace behind `/mcp` in `vellum serve`.
pub struct ServeWorkspace {
    drive: Arc<Drive>,
    index: Arc<dyn DriveIndex>,
    ops: HashMap<&'static str, Operation>,
    base_url: String,
    exporter: std::sync::OnceLock<Exporter>,
}

/// What an MCP export asks for (D265).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportWhat {
    /// One page.
    Page(ObjectId),
    /// A book, or the subtree under one of its sections.
    Book {
        /// The book.
        id: String,
        /// The section whose subtree to export; the whole book when `None`.
        section: Option<String>,
    },
}

/// One exported file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportFile {
    /// `<title>.<ext>`.
    pub file_name: String,
    /// The media type.
    pub mime: String,
    /// The file.
    pub bytes: Vec<u8>,
}

/// The server's exporter: runs one export (`pdf`, `docx` or `md`) as the
/// principal, through the same code as the export routes.
pub type Exporter = Arc<
    dyn Fn(
            vellum_contracts::Principal,
            ExportWhat,
            String,
        )
            -> vellum_contracts::BoxFuture<'static, Result<ExportFile, Error>>
        + Send
        + Sync,
>;

/// Largest export handed to the client inline; bigger files are fetched
/// from the export route with the same token.
const INLINE_EXPORT: usize = 4 * 1024 * 1024;

impl std::fmt::Debug for ServeWorkspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServeWorkspace")
            .field("base_url", &self.base_url)
            .field("ops", &self.ops.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl ServeWorkspace {
    /// The workspace over `drive` and the operations of `services`, with
    /// app links under `base_url`.
    #[must_use]
    pub fn new(
        drive: Arc<Drive>,
        index: Arc<dyn DriveIndex>,
        services: &[Arc<dyn Service>],
        base_url: &str,
    ) -> Self {
        let ops = services
            .iter()
            .flat_map(|s| Arc::clone(s).operations())
            .filter(|op| OPS.contains(&op.id()))
            .map(|op| (op.id(), op))
            .collect();
        Self {
            drive,
            index,
            ops,
            base_url: base_url.trim_end_matches('/').to_owned(),
            exporter: std::sync::OnceLock::new(),
        }
    }

    /// Serves `export` through `exporter` (the server's export routes).
    pub fn set_exporter(&self, exporter: Exporter) {
        let _ = self.exporter.set(exporter);
    }

    /// The route the same file downloads from, with the caller's token.
    fn export_url(&self, what: &ExportWhat, format: &str) -> String {
        match what {
            ExportWhat::Page(id) => {
                format!(
                    "{}/api/v1/export/pages/{id}?format={format}",
                    self.base_url
                )
            }
            ExportWhat::Book { id, section: None } => {
                format!(
                    "{}/api/v1/export/books/{id}?format={format}",
                    self.base_url
                )
            }
            ExportWhat::Book {
                id,
                section: Some(section),
            } => format!(
                "{}/api/v1/export/books/{id}?format={format}&scope=subtree&sect\
                    ion={section}",
                self.base_url
            ),
        }
    }

    /// Runs one service operation as `caller`.
    async fn call(
        &self,
        caller: &Caller,
        id: &str,
        input: Value,
    ) -> Result<Value, Error> {
        let op = self.ops.get(id).ok_or_else(|| {
            Error::new(
                ErrorKind::Unavailable,
                format!("{id} is not served here"),
            )
        })?;
        let ctx = RequestContext {
            request_id: RequestId::generate(),
            principal: caller.principal.clone(),
            deadline: Instant::now() + BUDGET,
            traceparent: None,
        };
        op.call(ctx, input).await
    }

    fn page_url(&self, id: impl std::fmt::Display) -> String {
        format!("{}/p/{id}", self.base_url)
    }

    /// The spaces the caller can open: their My Drive and shared drives.
    async fn spaces(
        &self,
        caller: &Caller,
    ) -> Result<Vec<Space>, WorkspaceError> {
        let mut spaces = self
            .drive
            .shared_drives(caller.user)
            .await
            .map_err(write_err)?;
        if let Some(mine) =
            self.drive.my_drive(caller.user).await.map_err(write_err)?
        {
            spaces.insert(0, mine);
        }
        Ok(spaces)
    }

    async fn space_by_slug(
        &self,
        caller: &Caller,
        slug: &str,
    ) -> Result<Space, WorkspaceError> {
        self.spaces(caller)
            .await?
            .into_iter()
            .find(|s| s.slug == slug || s.id.to_string() == slug)
            .ok_or_else(|| WorkspaceError::NotFound(slug.to_owned()))
    }

    async fn slug_of(&self, space: SpaceId) -> String {
        self.index
            .space(space)
            .await
            .ok()
            .flatten()
            .map(|s| s.slug)
            .unwrap_or_default()
    }

    /// A page with its blocks, as the caller may read it.
    async fn read_page(
        &self,
        caller: &Caller,
        id: &str,
    ) -> Result<(Document, PrivateDoc), WorkspaceError> {
        let oid = object_id(id)?;
        let (node, blob) = self
            .drive
            .read(&caller.model(), oid)
            .await
            .map_err(|e| read_err(id)(e.into()))?;
        if node.kind != NodeKind::Page {
            return Err(WorkspaceError::NotFound(format!(
                "{id} is not a page"
            )));
        }
        let markdown = String::from_utf8_lossy(&blob.bytes).into_owned();
        let doc = vellum_private::read_document(
            node.space,
            node.path.as_str(),
            markdown.clone(),
            node.modified,
        )
        .map_err(|e| WorkspaceError::Unavailable(e.to_string()))?;
        Ok((
            Document {
                id: node.id.to_string(),
                title: node.title.clone(),
                markdown,
                url: self.page_url(node.id),
                space: self.slug_of(node.space).await,
                version: blob.meta.etag.to_string(),
                updated_at: node.modified.to_string(),
            },
            doc,
        ))
    }

    fn receipt(&self, id: &str, mode: WriteMode) -> WriteReceipt {
        WriteReceipt {
            id: id.to_owned(),
            uri: format!("vellum://doc/{id}"),
            url: self.page_url(id),
            mode,
        }
    }

    /// Writes a draft page into the caller's My Drive.
    async fn draft(
        &self,
        caller: &Caller,
        title: &str,
        body: &str,
        front: Vec<(&'static str, String)>,
    ) -> Result<WriteReceipt, WorkspaceError> {
        let mine = self
            .drive
            .ensure_my_drive(caller.user, None)
            .await
            .map_err(write_err)?;
        let mut fields = vec![("status", "draft".to_owned())];
        fields.extend(front);
        let pairs: Vec<(&str, &str)> =
            fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let node = self
            .drive
            .create_page_with(
                &caller.model(),
                mine.id,
                None,
                title,
                body,
                &pairs,
            )
            .await
            .map_err(write_err)?;
        Ok(self.receipt(&node.id.to_string(), WriteMode::Draft))
    }

    /// Where a new page goes: `space:<slug>`, a space, a folder, or next
    /// to a page.
    async fn parent(
        &self,
        caller: &Caller,
        parent: &str,
    ) -> Result<(SpaceId, Option<ObjectId>), WorkspaceError> {
        if let Some(slug) = parent.strip_prefix("space:") {
            return Ok((self.space_by_slug(caller, slug).await?.id, None));
        }
        let id = object_id(parent)?;
        if let Some(space) = self
            .spaces(caller)
            .await?
            .into_iter()
            .find(|s| s.node_id() == id)
        {
            return Ok((space.id, None));
        }
        let node = self
            .drive
            .get(&caller.model(), id)
            .await
            .map_err(|e| read_err(parent)(e.into()))?;
        Ok(match node.kind {
            NodeKind::Folder => (node.space, Some(node.id)),
            _ => (node.space, node.parent),
        })
    }

    fn issue(&self, out: &Value) -> Issue {
        let key = text(out, "key");
        let mut front = format!(
            "---\nkey: {key}\nteam: {}\ntitle: {}\nstate: {}\npriority: {}\n",
            text(out, "team"),
            serde_json::to_string(&text(out, "title")).unwrap_or_default(),
            text(out, "state"),
            out.get("priority")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
        );
        for field in
            ["assignee", "project", "cycle", "parent", "due", "estimate"]
        {
            let value = text(out, field);
            if !value.is_empty() {
                let _ = writeln!(front, "{field}: {value}");
            }
        }
        if let Some(labels) = out
            .get("labels")
            .filter(|l| l.as_array().is_some_and(|a| !a.is_empty()))
        {
            let _ = writeln!(front, "labels: {labels}");
        }
        Issue {
            title: text(out, "title"),
            state: text(out, "state"),
            markdown: format!("{front}---\n\n{}\n", text(out, "description")),
            url: format!("{}/issues/{key}", self.base_url),
            key,
        }
    }

    async fn view(
        &self,
        caller: &Caller,
        input: Value,
    ) -> Result<Value, WorkspaceError> {
        let id = text(&input, "collection_id");
        self.call(caller, "collections.query", input)
            .await
            .map_err(|e| match e.kind() {
                ErrorKind::InvalidInput => write_error(&e),
                _ => read_err(&id)(e),
            })
    }

    async fn book_toc(
        &self,
        caller: &Caller,
        id: &str,
    ) -> Result<Value, WorkspaceError> {
        self.call(caller, "books.get", json!({ "book_id": id }))
            .await
            .map_err(read_err(id))
    }

    /// Page templates (D264): pages in a folder named `Templates` in any
    /// space the caller can open, by name, first title wins.
    async fn templates(
        &self,
        caller: &Caller,
    ) -> Result<Vec<(String, vellum_drive::Node)>, WorkspaceError> {
        let who = caller.model();
        let folders = self
            .drive
            .find(
                &who,
                &Filter {
                    title: Some(TEMPLATES.to_owned()),
                    kinds: vec![NodeKind::Folder],
                    ..Filter::default()
                },
                MAX_TEMPLATES,
            )
            .await
            .map_err(write_err)?;
        let mut out: Vec<(String, vellum_drive::Node)> = Vec::new();
        for folder in folders
            .iter()
            .filter(|f| f.title.eq_ignore_ascii_case(TEMPLATES))
        {
            let pages = self
                .drive
                .find(
                    &who,
                    &Filter {
                        kinds: vec![NodeKind::Page],
                        location: Some(folder.id),
                        ..Filter::default()
                    },
                    MAX_TEMPLATES,
                )
                .await
                .map_err(write_err)?;
            out.extend(pages.into_iter().map(|p| (template_name(&p.title), p)));
        }
        out.sort_by(|a, b| {
            a.0.cmp(&b.0).then_with(|| a.1.title.cmp(&b.1.title))
        });
        out.dedup_by(|a, b| a.0 == b.0);
        Ok(out)
    }

    /// The caller's activity since `from`, oldest first.
    async fn changes(
        &self,
        caller: &Caller,
        from: Timestamp,
    ) -> Result<Vec<Change>, WorkspaceError> {
        let who = caller.model();
        let mut all = Vec::new();
        for space in self.spaces(caller).await? {
            let Ok(entries) = self
                .drive
                .activity(&who, space.node_id(), ACTIVITY_PER_SPACE)
                .await
            else {
                continue;
            };
            all.extend(entries.into_iter().filter(|a| a.at >= from));
        }
        all.sort_by_key(|a| a.at);
        let mut titles: BTreeMap<ObjectId, Option<String>> = BTreeMap::new();
        let mut out = Vec::with_capacity(all.len());
        for a in all {
            if let std::collections::btree_map::Entry::Vacant(slot) =
                titles.entry(a.node)
            {
                let _ = slot.insert(
                    self.drive.get(&who, a.node).await.ok().map(|n| n.title),
                );
            }
            // Items the caller cannot see (or that are gone) are skipped.
            let Some(Some(title)) = titles.get(&a.node).cloned() else {
                continue;
            };
            out.push(Change {
                uri: format!("vellum://doc/{}", a.node),
                title,
                action: action_name(&a.kind).to_owned(),
                actor: actor_name(a.actor),
                at: a.at.to_string(),
            });
        }
        Ok(out)
    }
}

const fn action_name(kind: &vellum_drive::ActivityKind) -> &'static str {
    use vellum_drive::ActivityKind as K;
    match kind {
        K::Created => "created",
        K::Edited { .. } => "edited",
        K::Renamed { .. } => "renamed",
        K::Moved { .. } => "moved",
        K::Copied { .. } => "copied",
        K::Trashed => "trashed",
        K::Restored => "restored",
        _ => "changed",
    }
}

fn actor_name(actor: Who) -> String {
    match actor {
        Who::User { user } => format!("user:{user}"),
        Who::Model { on_behalf_of } => format!("model for user:{on_behalf_of}"),
        Who::External => "an edit outside Vellum".to_owned(),
        _ => "Vellum".to_owned(),
    }
}

fn book_markdown(info: &Value) -> String {
    fn chapter(out: &mut String, node: &Value, depth: usize) {
        let number = text(node, "number");
        let label = if number.is_empty() {
            text(node, "title")
        } else {
            format!("{number} {}", text(node, "title"))
        };
        let id = text(node, "id");
        let line = if id.is_empty() {
            label
        } else {
            format!("[{label}](vellum://doc/{id})")
        };
        let _ = writeln!(out, "{}- {line}", "  ".repeat(depth));
        for child in node
            .get("children")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            chapter(out, child, depth + 1);
        }
    }
    let mut out = format!("# {}\n\n", text(info, "title"));
    for line in info
        .get("toc")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match text(line, "kind").as_str() {
            "chapter" => chapter(&mut out, line, 0),
            "part" => {
                let _ = writeln!(out, "\n## {}\n", text(line, "title"));
            }
            _ => out.push_str("\n---\n\n"),
        }
    }
    out
}

fn book_outline(info: &Value) -> Vec<OutlineNode> {
    fn node(v: &Value) -> OutlineNode {
        let number = text(v, "number");
        OutlineNode {
            id: text(v, "id"),
            number: (!number.is_empty()).then_some(number),
            title: text(v, "title"),
            depth: v
                .get("depth")
                .and_then(Value::as_u64)
                .and_then(|d| u8::try_from(d).ok())
                .unwrap_or(1),
            children: v
                .get("children")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(node)
                .collect(),
        }
    }
    info.get("toc")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|l| text(l, "kind") == "chapter")
        .map(node)
        .collect()
}

/// Rows of a view result, flattened to `id`, `title` and one field per
/// column name.
fn rows_of(result: &Value) -> (Vec<String>, Vec<Value>) {
    let columns: Vec<String> = result
        .get("columns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| text(c, "name"))
        .collect();
    let mut rows = Vec::new();
    for group in result
        .get("groups")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for row in group
            .get("rows")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let mut out = Map::new();
            let _ = out.insert("id".into(), json!(text(row, "id")));
            let _ = out.insert("title".into(), json!(text(row, "title")));
            let cells = row
                .get("cells")
                .and_then(Value::as_array)
                .into_iter()
                .flatten();
            for (name, cell) in columns.iter().zip(cells) {
                let value = cell
                    .get("value")
                    .cloned()
                    .unwrap_or_else(|| json!(text(cell, "display")));
                let _ = out.insert(name.clone(), value);
            }
            rows.push(Value::Object(out));
        }
    }
    (columns, rows)
}

fn table(columns: &[String], rows: &[Value]) -> String {
    let cell = |v: Option<&Value>| -> String {
        let s = match v {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => String::new(),
            Some(other) => other.to_string(),
        };
        s.replace('|', "\\|").replace('\n', " ")
    };
    let mut out = format!(
        "| {} |\n|{}\n",
        columns.join(" | "),
        " --- |".repeat(columns.len())
    );
    for row in rows {
        let cells: Vec<String> =
            columns.iter().map(|c| cell(row.get(c))).collect();
        let _ = writeln!(out, "| {} |", cells.join(" | "));
    }
    out
}

#[async_trait]
impl Workspace for ServeWorkspace {
    async fn search(
        &self,
        actor: &Actor,
        query: SearchQuery,
    ) -> Result<Vec<SearchHit>, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let mut input =
            json!({ "q": query.query, "limit": query.limit.clamp(1, 100) });
        if let Some(slug) = &query.space {
            put(
                &mut input,
                "spaces",
                json!(self.space_by_slug(&caller, slug).await?.id.to_string()),
            );
        }
        if let Some(kind) = &query.kind {
            put(&mut input, "kinds", json!(kind));
        }
        if let Some(owner) = query
            .owner
            .as_deref()
            .filter(|o| o.parse::<UserId>().is_ok())
        {
            put(&mut input, "owners", json!(owner));
        }
        if let Some(from) = &query.modified_after {
            put(&mut input, "modified_from", json!(since(from)?.unix_ms()));
        }
        if let Some(to) = &query.modified_before {
            put(&mut input, "modified_to", json!(since(to)?.unix_ms()));
        }
        let page: vellum_search::SearchPage = serde_json::from_value(
            self.call(&caller, "search.query", input)
                .await
                .map_err(|e| write_error(&e))?,
        )
        .map_err(|e| WorkspaceError::Unavailable(e.to_string()))?;
        Ok(page
            .hits
            .into_iter()
            .map(|hit| SearchHit {
                doc_id: hit.doc.to_string(),
                block_id: Some(hit.block.to_string()),
                title: hit.title,
                heading_path: hit.heading_path,
                snippet: hit
                    .snippet
                    .iter()
                    .map(|f| {
                        if f.hit {
                            format!("**{}**", f.text)
                        } else {
                            f.text.clone()
                        }
                    })
                    .collect(),
                citation_url: hit.citation,
                score: hit.score,
            })
            .collect())
    }

    async fn get_doc(
        &self,
        actor: &Actor,
        id: &str,
    ) -> Result<Document, WorkspaceError> {
        let caller = Caller::of(actor)?;
        Ok(self.read_page(&caller, id).await?.0)
    }

    async fn get_doc_by_path(
        &self,
        actor: &Actor,
        path: &str,
    ) -> Result<Document, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let not_found = || WorkspaceError::NotFound(path.to_owned());
        // The app's own page URL, `/p/{id}`.
        if let Some(id) = path
            .strip_prefix("/p/")
            .map(|rest| rest.trim_end_matches('/'))
            .filter(|id| !id.is_empty() && !id.contains('/'))
        {
            return self.get_doc(actor, id).await;
        }
        let (slug, rest) = path
            .trim_start_matches('/')
            .split_once('/')
            .ok_or_else(not_found)?;
        let space = self.space_by_slug(&caller, slug).await?;
        let file = space.root.join(rest).map_err(|_| not_found())?;
        let node = self
            .index
            .node_at(space.id, &file)
            .await
            .ok()
            .flatten()
            .ok_or_else(not_found)?;
        self.get_doc(actor, &node.id.to_string()).await
    }

    async fn get_blocks(
        &self,
        actor: &Actor,
        doc_id: &str,
        selector: BlockSelector,
    ) -> Result<Vec<Block>, WorkspaceError> {
        let caller = Caller::of(actor)?;
        select(
            blocks_of(&self.read_page(&caller, doc_id).await?.1),
            selector,
        )
    }

    async fn get_outline(
        &self,
        actor: &Actor,
        id: &str,
    ) -> Result<Vec<OutlineNode>, WorkspaceError> {
        let caller = Caller::of(actor)?;
        match self.read_page(&caller, id).await {
            Ok((_, doc)) => Ok(nest(
                blocks_of(&doc)
                    .into_iter()
                    .filter(|b| b.kind == "heading")
                    .map(|b| OutlineNode {
                        id: b.id,
                        number: None,
                        title: b
                            .heading_path
                            .last()
                            .cloned()
                            .unwrap_or_default(),
                        depth: u8::try_from(b.heading_path.len())
                            .unwrap_or(u8::MAX),
                        children: Vec::new(),
                    })
                    .collect(),
            )),
            Err(WorkspaceError::NotFound(_)) => {
                Ok(book_outline(&self.book_toc(&caller, id).await?))
            }
            Err(e) => Err(e),
        }
    }

    async fn get_book(
        &self,
        actor: &Actor,
        id: &str,
    ) -> Result<Document, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let info = self.book_toc(&caller, id).await?;
        Ok(Document {
            id: id.to_owned(),
            title: text(&info, "title"),
            markdown: book_markdown(&info),
            url: format!("{}/books/{id}", self.base_url),
            space: String::new(),
            version: String::new(),
            updated_at: String::new(),
        })
    }

    async fn space_tree(
        &self,
        actor: &Actor,
        slug: &str,
    ) -> Result<Document, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let space = self.space_by_slug(&caller, slug).await?;
        let tree = self
            .drive
            .tree(
                &caller.model(),
                space.id,
                None,
                None,
                vellum_drive::MAX_TREE_ITEMS,
            )
            .await
            .map_err(|e| read_err(slug)(e.into()))?;
        let mut children: HashMap<Option<ObjectId>, Vec<&vellum_drive::Node>> =
            HashMap::new();
        for node in &tree.items {
            children.entry(node.parent).or_default().push(node);
        }
        let mut out = format!("# {}\n\n", space.name);
        let mut stack: Vec<(usize, &vellum_drive::Node)> = children
            .get(&None)
            .into_iter()
            .flatten()
            .rev()
            .map(|n| (0, *n))
            .collect();
        while let Some((depth, node)) = stack.pop() {
            let line = match node.kind {
                NodeKind::Page => {
                    format!("[{}](vellum://doc/{})", node.title, node.id)
                }
                NodeKind::Book => {
                    format!("[{}](vellum://book/{})", node.title, node.id)
                }
                NodeKind::Collection => {
                    format!("[{}](vellum://collection/{})", node.title, node.id)
                }
                _ => format!("{}/", node.title),
            };
            let _ = writeln!(out, "{}- {line}", "  ".repeat(depth));
            if let Some(kids) = children.get(&Some(node.id)) {
                stack.extend(kids.iter().rev().map(|n| (depth + 1, *n)));
            }
        }
        if tree.truncated {
            out.push_str(
                "\n(The tree is longer than this; search to find the rest.)\n",
            );
        }
        Ok(Document {
            id: space.id.to_string(),
            title: space.name.clone(),
            markdown: out,
            url: format!("vellum://space/{}/tree", space.slug),
            space: space.slug,
            version: String::new(),
            updated_at: String::new(),
        })
    }

    async fn create_page(
        &self,
        actor: &Actor,
        page: NewPage,
        mode: WriteMode,
        by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError> {
        let caller = Caller::of(actor)?;
        if page.title.trim().is_empty() {
            return Err(WorkspaceError::Invalid("the title is empty".into()));
        }
        let (space, parent) = self.parent(&caller, &page.parent).await?;
        let mut fields = Vec::new();
        if mode == WriteMode::Draft {
            fields.push(("status", "draft".to_owned()));
        }
        fields.extend(attribution(&by));
        if let Some(template) = &page.template {
            fields.push(("template", template.clone()));
        }
        let pairs: Vec<(&str, &str)> =
            fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let node = self
            .drive
            .create_page_with(
                &caller.model(),
                space,
                parent,
                &page.title,
                &page.markdown,
                &pairs,
            )
            .await
            .map_err(write_err)?;
        Ok(self.receipt(&node.id.to_string(), mode))
    }

    async fn edit_page(
        &self,
        actor: &Actor,
        edit: EditProposal,
        mode: WriteMode,
        by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let (doc, parsed) = self.read_page(&caller, &edit.doc_id).await?;
        if edit
            .base_version
            .as_ref()
            .is_some_and(|v| *v != doc.version)
        {
            return Err(WorkspaceError::Conflict(format!(
                "the page changed since version {}; it is now at {}",
                edit.base_version.unwrap_or_default(),
                doc.version
            )));
        }
        let ids: Vec<String> =
            parsed.blocks.iter().map(|b| b.id.to_string()).collect();
        let edited = crate::mcp_edit::apply(&doc.markdown, &ids, &edit.edits)?;
        if mode == WriteMode::Draft {
            let mut front = vec![
                ("draft_of", doc.id.clone()),
                ("base_version", doc.version.clone()),
                ("summary", edit.summary.clone()),
            ];
            front.extend(attribution(&by));
            let title = format!("Proposed edit: {}", doc.title);
            return self.draft(&caller, &title, &body_of(&edited), front).await;
        }
        let base = Base {
            bytes: Bytes::from(doc.markdown.clone()),
            etag: Some(vellum_store::Etag::new(doc.version.clone())),
        };
        let outcome = self
            .drive
            .save(
                &caller.model(),
                object_id(&doc.id)?,
                base,
                Bytes::from(edited),
            )
            .await
            .map_err(write_err)?;
        match outcome.saved {
            Saved::Conflicted { copy } => {
                Err(WorkspaceError::Conflict(format!(
                    "someone changed the same blocks; the edit was saved as a \
                        copy, {}",
                    self.page_url(copy.id)
                )))
            }
            _ => Ok(self.receipt(&doc.id, WriteMode::Direct)),
        }
    }

    async fn get_issue(
        &self,
        actor: &Actor,
        key: &str,
    ) -> Result<Issue, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let out = self
            .call(&caller, "work.get_issue", json!({ "key": key }))
            .await
            .map_err(read_err(key))?;
        Ok(self.issue(&out))
    }

    async fn create_issue(
        &self,
        actor: &Actor,
        issue: NewIssue,
        _by: Attribution,
    ) -> Result<Issue, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let input = json!({
            "team": issue.team,
            "title": issue.title,
            "description": issue.description,
            "priority": issue.priority,
            "labels": issue.labels,
            "assignee": issue.assignee,
            "project": issue.project,
            "parent": issue.parent,
            "source_block": issue.source_block,
            // A model's issue goes to the team's triage, like other intake.
            "intake": true,
        });
        let out = self
            .call(&caller, "work.create_issue", input)
            .await
            .map_err(|e| write_error(&e))?;
        Ok(self.issue(&out))
    }

    async fn update_issue(
        &self,
        actor: &Actor,
        update: IssueUpdate,
        _by: Attribution,
    ) -> Result<Issue, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let input = json!({
            "key": update.key,
            "title": update.title,
            "state": update.state,
            "priority": update.priority,
            "assignee": update.assignee,
            "add_labels": update.add_labels,
            "remove_labels": update.remove_labels,
            "estimate": update.estimate,
            "due": update.due,
        });
        let out = self
            .call(&caller, "work.update_issue", input)
            .await
            .map_err(|e| write_error(&e))?;
        Ok(self.issue(&out))
    }

    async fn add_comment(
        &self,
        actor: &Actor,
        comment: NewComment,
        _by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let target = comment.reply_to.is_none().then(|| comment.target.clone());
        let out = self
            .call(
                &caller,
                "work.add_comment",
                json!({
                    "target": target,
                    "reply_to": comment.reply_to,
                    "body": comment.markdown,
                    "by_model": true,
                }),
            )
            .await
            .map_err(|e| write_error(&e))?;
        let on = comment.target.split('#').next().unwrap_or_default();
        let (uri, url) = if on.parse::<ObjectId>().is_ok() {
            (format!("vellum://doc/{on}"), self.page_url(on))
        } else {
            (
                format!("vellum://issue/{on}"),
                format!("{}/issues/{on}", self.base_url),
            )
        };
        Ok(WriteReceipt {
            id: text(&out, "id"),
            uri,
            url,
            mode: WriteMode::Direct,
        })
    }

    async fn create_task(
        &self,
        actor: &Actor,
        task: NewTask,
        _by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let mut line = task.title.clone();
        if let Some(assignee) = &task.assignee {
            let _ =
                write!(line, " +{}", assignee.trim_start_matches(['@', '+']));
        }
        let out = self
            .call(
                &caller,
                "tasks.create",
                json!({ "text": line, "list": task.list }),
            )
            .await
            .map_err(|e| write_error(&e))?;
        let id = text(&out, "id");
        if task.start_date.is_some() || task.due.is_some() {
            let _ = self
                .call(
                    &caller,
                    "tasks.update",
                    json!({
                        "task": id,
                        "start": task.start_date,
                        "due": task.due
                    }),
                )
                .await
                .map_err(|e| write_error(&e))?;
        }
        Ok(WriteReceipt {
            uri: format!("vellum://task/{id}"),
            url: format!("{}/my-work", self.base_url),
            id,
            mode: WriteMode::Direct,
        })
    }

    async fn query_collection(
        &self,
        actor: &Actor,
        query: CollectionQuery,
    ) -> Result<RowPage, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let filter = query
            .filters
            .iter()
            .map(filter_text)
            .collect::<Result<Vec<_>, _>>()?;
        let sort: Vec<String> = query.sorts.iter().map(sort_text).collect();
        let result = self
            .view(
                &caller,
                json!({
                    "collection_id": query.collection,
                    "view": query.view,
                    "filter": filter,
                    "sort": sort,
                    "limit": query.limit.clamp(1, 500),
                }),
            )
            .await?;
        let (columns, rows) = rows_of(&result);
        Ok(RowPage {
            columns,
            rows,
            next_cursor: None,
        })
    }

    async fn get_collection(
        &self,
        actor: &Actor,
        id: &str,
        view: Option<&str>,
    ) -> Result<Document, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let result = self
            .view(
                &caller,
                json!({ "collection_id": id, "view": view, "limit": 200 }),
            )
            .await?;
        let (columns, rows) = rows_of(&result);
        let mut names = vec!["title".to_owned()];
        names.extend(
            columns
                .into_iter()
                .filter(|c| !c.eq_ignore_ascii_case("title")),
        );
        let title = text(&result, "title");
        Ok(Document {
            id: id.to_owned(),
            markdown: format!("# {title}\n\n{}", table(&names, &rows)),
            title,
            url: format!("{}/data/{id}", self.base_url),
            space: String::new(),
            version: String::new(),
            updated_at: String::new(),
        })
    }

    async fn upsert_rows(
        &self,
        actor: &Actor,
        upsert: RowUpsert,
        mode: WriteMode,
        by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let collection = upsert.collection.clone();
        let receipt = |mode| WriteReceipt {
            id: collection.clone(),
            uri: format!("vellum://collection/{collection}"),
            url: format!("{}/data/{collection}", self.base_url),
            mode,
        };
        if mode == WriteMode::Draft {
            let info = self
                .call(
                    &caller,
                    "collections.get",
                    json!({ "collection_id": collection }),
                )
                .await
                .map_err(read_err(&collection))?;
            let rows =
                serde_json::to_string_pretty(&upsert.rows).unwrap_or_default();
            let mut front =
                vec![("draft_of", format!("collection:{collection}"))];
            front.extend(attribution(&by));
            let title = format!("Proposed rows: {}", text(&info, "title"));
            let body = format!(
                "Rows proposed for [{}](vellum://collection/{collection}): \
                    rows with an `id` change that row, the others are new.\n\n\
                    ```json\n{rows}\n```\n",
                text(&info, "title")
            );
            return self.draft(&caller, &title, &body, front).await;
        }
        for row in upsert.rows {
            let Value::Object(mut values) = row else {
                return Err(WorkspaceError::Invalid(
                    "each row is an object".into(),
                ));
            };
            let id = values.remove("id");
            let body = values.remove("body");
            let input = match id {
                Some(id) => {
                    json!({
                        "collection_id": collection,
                        "row_id": id,
                        "values": values,
                        "body": body
                    })
                }
                None => {
                    json!({
                        "collection_id": collection,
                        "values": values,
                        "body": body.unwrap_or_else(|| json!(""))
                    })
                }
            };
            let op = if input.get("row_id").is_some() {
                "collections.update_row"
            } else {
                "collections.create_row"
            };
            let _ = self
                .call(&caller, op, input)
                .await
                .map_err(|e| write_error(&e))?;
        }
        Ok(receipt(WriteMode::Direct))
    }

    async fn export(
        &self,
        actor: &Actor,
        request: ExportRequest,
    ) -> Result<ExportArtifact, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let Some(exporter) = self.exporter.get() else {
            return Err(WorkspaceError::Unavailable(
                "export is not offered over MCP on this server yet; read the \
                    page with get_doc instead"
                    .into(),
            ));
        };
        if !matches!(request.format.as_str(), "pdf" | "docx" | "md") {
            return Err(WorkspaceError::Invalid(
                "format must be pdf, docx or md".into(),
            ));
        }
        let page = match request.target.parse::<ObjectId>() {
            Ok(id) => self
                .drive
                .get(&caller.model(), id)
                .await
                .ok()
                .filter(|n| n.kind == NodeKind::Page)
                .map(|n| n.id),
            Err(_) => None,
        };
        let what = page.map_or_else(
            || ExportWhat::Book {
                id: request.target.clone(),
                section: request.section.clone(),
            },
            ExportWhat::Page,
        );
        let file = exporter(
            caller.principal.clone(),
            what.clone(),
            request.format.clone(),
        )
        .await
        .map_err(read_err(&request.target))?;
        let size = u64::try_from(file.bytes.len()).unwrap_or(u64::MAX);
        let data = (file.bytes.len() <= INLINE_EXPORT).then(|| {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(&file.bytes)
        });
        Ok(ExportArtifact {
            url: self.export_url(&what, &request.format),
            name: file.file_name,
            mime_type: file.mime,
            size,
            data,
        })
    }

    async fn list_changes(
        &self,
        actor: &Actor,
        from: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<ChangePage, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let all = self.changes(&caller, since(from)?).await?;
        let (changes, next_cursor) = page(all, cursor.as_deref(), limit)?;
        Ok(ChangePage {
            changes,
            next_cursor,
        })
    }

    async fn list_readable(
        &self,
        actor: &Actor,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<ListingPage, WorkspaceError> {
        let Ok(caller) = Caller::of(actor) else {
            // Nothing is public through MCP: public pages have their own site.
            return Ok(ListingPage::default());
        };
        let spaces = self.spaces(&caller).await?;
        let slugs: HashMap<SpaceId, String> =
            spaces.iter().map(|s| (s.id, s.slug.clone())).collect();
        let mut items: Vec<Listing> = spaces
            .into_iter()
            .map(|s| Listing {
                uri: format!("vellum://space/{}/tree", s.slug),
                title: s.name.clone(),
                description: None,
                url: self.base_url.clone(),
                space: s.slug,
            })
            .collect();
        // Every page the caller may view, newest first (D295).
        let pages = self
            .drive
            .find(
                &caller.model(),
                &vellum_drive::Filter {
                    kinds: vec![NodeKind::Page],
                    ..vellum_drive::Filter::default()
                },
                LISTED_PAGES,
            )
            .await
            .map_err(write_err)?;
        items.extend(pages.into_iter().map(|node| Listing {
            uri: format!("vellum://doc/{}", node.id),
            title: node.title.clone(),
            description: None,
            url: self.page_url(node.id),
            space: slugs.get(&node.space).cloned().unwrap_or_default(),
        }));
        if let Ok(books) = self.call(&caller, "books.list", json!({})).await {
            for b in books
                .get("books")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let id = text(b, "id");
                items.push(Listing {
                    uri: format!("vellum://book/{id}"),
                    title: text(b, "title"),
                    description: None,
                    url: format!("{}/books/{id}", self.base_url),
                    space: String::new(),
                });
            }
        }
        if let Ok(list) =
            self.call(&caller, "collections.list", json!({})).await
        {
            for c in list
                .get("collections")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let id = text(c, "id");
                items.push(Listing {
                    uri: format!("vellum://collection/{id}"),
                    title: text(c, "title"),
                    description: None,
                    url: format!("{}/data/{id}", self.base_url),
                    space: String::new(),
                });
            }
        }
        let (items, next_cursor) = page(items, cursor.as_deref(), limit)?;
        Ok(ListingPage { items, next_cursor })
    }

    async fn list_templates(
        &self,
        actor: &Actor,
    ) -> Result<Vec<Template>, WorkspaceError> {
        let Ok(caller) = Caller::of(actor) else {
            return Ok(Vec::new());
        };
        Ok(self
            .templates(&caller)
            .await?
            .into_iter()
            .map(|(name, node)| Template {
                uri: format!("vellum://template/{name}"),
                name,
                title: node.title,
            })
            .collect())
    }

    async fn get_template(
        &self,
        actor: &Actor,
        name: &str,
    ) -> Result<String, WorkspaceError> {
        let caller = Caller::of(actor)?;
        let wanted = template_name(name);
        let (_, node) = self
            .templates(&caller)
            .await?
            .into_iter()
            .find(|(n, _)| *n == wanted)
            .ok_or_else(|| WorkspaceError::NotFound(name.to_owned()))?;
        let (doc, _) = self.read_page(&caller, &node.id.to_string()).await?;
        Ok(body_of(&doc.markdown))
    }

    async fn site_info(&self) -> Result<SiteInfo, WorkspaceError> {
        Ok(SiteInfo {
            name: "Vellum".into(),
            summary: "A Vellum workspace: documents, books, collections, \
                issues and tasks, each a Markdown file."
                .into(),
            base_url: self.base_url.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_nest_by_depth() {
        let flat = |id: &str, depth| OutlineNode {
            id: id.into(),
            number: None,
            title: id.into(),
            depth,
            children: Vec::new(),
        };
        let tree = nest(vec![
            flat("a", 2),
            flat("b", 3),
            flat("c", 3),
            flat("d", 2),
            flat("e", 4),
        ]);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].children.len(), 2);
        assert_eq!(tree[1].children[0].id, "e");
    }

    #[test]
    fn filters_and_sorts_become_view_text() {
        assert_eq!(
            filter_text(
                &json!({ "property": "Status", "op": "neq", "value": "Done" })
            )
            .unwrap(),
            "\"Status\" != Done"
        );
        assert_eq!(
            filter_text(&json!({ "property": "Owner", "op": "empty" }))
                .unwrap(),
            "\"Owner\" is empty"
        );
        assert!(
            filter_text(&json!({ "property": "x", "op": "drop table" }))
                .is_err()
        );
        assert!(filter_text(&json!({ "op": "eq" })).is_err());
        assert_eq!(
            sort_text(&json!({ "property": "Due", "direction": "DESC" })),
            "\"Due\" desc"
        );
    }

    #[test]
    fn templates_are_named_from_their_titles() {
        assert_eq!(template_name("PRD"), "prd");
        assert_eq!(template_name("Weekly status: team"), "weekly-status-team");
        assert_eq!(template_name("  Post-mortem  "), "post-mortem");
    }

    #[test]
    fn pages_and_dates() {
        let (items, next) = page((0..5).collect(), None, 2).unwrap();
        assert_eq!((items, next.as_deref()), (vec![0, 1], Some("2")));
        let (items, next) = page((0..5).collect(), Some("4"), 2).unwrap();
        assert_eq!((items, next), (vec![4], None));
        assert!(page::<u8>(Vec::new(), Some("x"), 2).is_err());
        assert_eq!(
            since("2026-10-29").unwrap().to_string(),
            "2026-10-29T00:00:00Z"
        );
        assert!(since("yesterday").is_err());
    }

    #[test]
    fn view_rows_flatten_to_named_columns() {
        let result = json!({
            "title": "Roadmap",
            "columns": [
                { "key": "title", "name": "Name" },
                { "key": "s", "name": "Status" }
            ],
            "groups": [{ "rows": [{ "id": "r1", "title": "Ship", "cells": [
                { "display": "Ship" }, { "value": "Done", "display": "Done" }
            ] }] }]
        });
        let (columns, rows) = rows_of(&result);
        assert_eq!(columns, ["Name", "Status"]);
        assert_eq!(rows[0]["Status"], "Done");
        assert_eq!(rows[0]["id"], "r1");
        assert!(table(&columns, &rows).contains("| Ship | Done |"));
    }

    #[test]
    fn books_render_as_nested_lists() {
        let info = json!({ "title": "Handbook", "toc": [
            { "kind": "chapter", "id": "01JD0000000000000000000001",
              "title": "Intro", "number": "1", "depth": 1,
              "children": [{
                  "id": null,
                  "title": "Why",
                  "number": "1.1",
                  "depth": 2,
                  "children": []
              }] },
            { "kind": "part", "title": "Reference" },
        ]});
        let md = book_markdown(&info);
        assert!(
            md.contains(
                "- [1 \
                Intro](vellum://doc/01JD0000000000000000000001)\n  - 1.1 \
                Why\n"
            ),
            "{md}"
        );
        assert!(md.contains("## Reference"));
        let outline = book_outline(&info);
        assert_eq!(outline[0].children[0].number.as_deref(), Some("1.1"));
    }
}
