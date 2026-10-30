// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Ports: what the MCP server needs from the rest of Vellum.
//!
//! The server is an edge crate. It never reads another service's tables;
//! the application crate implements these traits by calling the pages,
//! drive, books, work, tasks, collections, search, export and audit
//! services. Every method receives the [`Actor`] so the implementation
//! applies the same authorization as the web UI: the model never sees
//! what the user cannot.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::Principal;

/// Who is asking: a user through a client, or an anonymous visitor (for
/// `llms.txt`, the Markdown sitemap and `?format=md` on public pages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Actor {
    /// An authenticated user acting through an MCP client.
    User(Principal),
    /// Nobody signed in; only public pages are visible.
    Anonymous,
}

/// Errors a workspace port reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkspaceError {
    /// The object does not exist or the actor may not see it. The two are
    /// deliberately indistinguishable.
    #[error("not found: {0}")]
    NotFound(String),
    /// The actor can see the object but may not change it.
    #[error("permission denied: {0}")]
    Forbidden(String),
    /// The input was rejected by the domain (bad filter, unknown field).
    #[error("invalid input: {0}")]
    Invalid(String),
    /// A concurrent change won; the caller should re-read and retry.
    #[error("conflict: {0}")]
    Conflict(String),
    /// A dependency is down; the call may be retried.
    #[error("temporarily unavailable: {0}")]
    Unavailable(String),
}

/// How a write is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    /// Lands as a draft for the user to review (the default).
    Draft,
    /// Applied directly; needs `write:direct`.
    Direct,
}

/// Attribution recorded on every model-made write, in front matter and
/// in the audit log: `authored_by: model`, `on_behalf_of: <user>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribution {
    /// Always `model` for MCP writes.
    pub authored_by: String,
    /// The user id the model acts for.
    pub on_behalf_of: String,
    /// The MCP client that made the call.
    pub client: String,
}

/// Search filters and paging.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Free text.
    pub query: String,
    /// Space slug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    /// Object type: page, book, issue, row, task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Page status: draft, in-review, verified, outdated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Owner user id or handle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Modified on or after this date (RFC 3339 or `YYYY-MM-DD`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_after: Option<String>,
    /// Modified before this date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_before: Option<String>,
    /// Maximum hits.
    pub limit: u32,
}

/// One block-level search hit with its citation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    /// Document id (ULID).
    pub doc_id: String,
    /// Block id (`blk-xxxxxxxx`), when the hit is a block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    /// Document title.
    pub title: String,
    /// Heading path of the section that holds the block.
    pub heading_path: Vec<String>,
    /// The matching text, as Vellum Markdown.
    pub snippet: String,
    /// A URL that opens the block in the app.
    pub citation_url: String,
    /// Fused rank score.
    pub score: f64,
}

/// A page as Vellum Markdown with front matter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
    /// Document id (ULID).
    pub id: String,
    /// Title.
    pub title: String,
    /// The whole file: front matter and body.
    pub markdown: String,
    /// App URL.
    pub url: String,
    /// Space slug.
    pub space: String,
    /// Opaque version (`ETag`).
    pub version: String,
    /// Last modified, RFC 3339.
    pub updated_at: String,
}

/// One block of a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    /// Block id.
    pub id: String,
    /// Block kind (paragraph, heading, list, container name, fence name).
    pub kind: String,
    /// The block as Vellum Markdown.
    pub markdown: String,
    /// Heading path of the enclosing section.
    pub heading_path: Vec<String>,
}

/// Which blocks to read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockSelector {
    /// Specific block ids.
    #[serde(default)]
    pub ids: Vec<String>,
    /// First block of a range (inclusive).
    #[serde(default)]
    pub from: Option<String>,
    /// Last block of a range (inclusive).
    #[serde(default)]
    pub to: Option<String>,
    /// A heading block id: return its whole section.
    #[serde(default)]
    pub section: Option<String>,
}

/// One node of an outline (headings of a page, or sections of a book).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutlineNode {
    /// Block id of the heading, or the node's document id in a book.
    pub id: String,
    /// Computed number such as `2.3`, when numbered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    /// Title.
    pub title: String,
    /// Depth from 1.
    pub depth: u8,
    /// Children.
    pub children: Vec<Self>,
}

/// A new page written by a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPage {
    /// Parent page, folder, book or space (id or `space:<slug>`).
    pub parent: String,
    /// Title.
    pub title: String,
    /// Body as Vellum Markdown (without front matter).
    pub markdown: String,
    /// Optional template name the draft was produced from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
}

/// One block-level change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum BlockEdit {
    /// Replace a block.
    Replace {
        /// Block id.
        block_id: String,
        /// New Markdown.
        markdown: String,
    },
    /// Insert after a block (or at the start when `after` is absent).
    Insert {
        /// Anchor block id.
        #[serde(default)]
        after: Option<String>,
        /// New Markdown.
        markdown: String,
    },
    /// Delete a block.
    Delete {
        /// Block id.
        block_id: String,
    },
}

/// A proposed edit of an existing page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditProposal {
    /// Document id.
    pub doc_id: String,
    /// The version the edit is based on, for conflict detection.
    #[serde(default)]
    pub base_version: Option<String>,
    /// The changes, applied in order.
    pub edits: Vec<BlockEdit>,
    /// A one-line summary shown to the reviewer.
    pub summary: String,
}

/// What a write produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteReceipt {
    /// Id of the written object (the draft's id for a draft).
    pub id: String,
    /// `vellum://` URI of the object.
    pub uri: String,
    /// App URL; for a draft, the review (diff) page.
    pub url: String,
    /// How the write was applied.
    pub mode: WriteMode,
}

/// A new issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewIssue {
    /// Team key such as `ENG`.
    pub team: String,
    /// Title.
    pub title: String,
    /// Description as Vellum Markdown.
    #[serde(default)]
    pub description: String,
    /// Priority 0–4.
    #[serde(default)]
    pub priority: Option<u8>,
    /// Labels.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Assignee handle.
    #[serde(default)]
    pub assignee: Option<String>,
    /// Project id.
    #[serde(default)]
    pub project: Option<String>,
    /// Parent issue key.
    #[serde(default)]
    pub parent: Option<String>,
    /// Source block `doc_id#blk-…` when filed from a page.
    #[serde(default)]
    pub source_block: Option<String>,
}

/// Changes to an issue; absent fields are left alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueUpdate {
    /// Issue key such as `ENG-142`.
    pub key: String,
    /// New title.
    #[serde(default)]
    pub title: Option<String>,
    /// New state name (a team status).
    #[serde(default)]
    pub state: Option<String>,
    /// New priority.
    #[serde(default)]
    pub priority: Option<u8>,
    /// New assignee handle; empty string unassigns.
    #[serde(default)]
    pub assignee: Option<String>,
    /// Labels to add.
    #[serde(default)]
    pub add_labels: Vec<String>,
    /// Labels to remove.
    #[serde(default)]
    pub remove_labels: Vec<String>,
    /// New estimate.
    #[serde(default)]
    pub estimate: Option<String>,
    /// New due date.
    #[serde(default)]
    pub due: Option<String>,
}

/// An issue as returned to the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    /// Key such as `ENG-142`.
    pub key: String,
    /// Title.
    pub title: String,
    /// Status name.
    pub state: String,
    /// The whole issue file (front matter and description).
    pub markdown: String,
    /// App URL.
    pub url: String,
}

/// A new comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewComment {
    /// Target: a document id, `doc_id#blk-…`, or an issue key.
    pub target: String,
    /// Body as Vellum Markdown.
    pub markdown: String,
    /// Parent comment id, to reply in a thread.
    #[serde(default)]
    pub reply_to: Option<String>,
}

/// A new personal task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewTask {
    /// Title; may contain quick-add tokens.
    pub title: String,
    /// List name; the Inbox when absent.
    #[serde(default)]
    pub list: Option<String>,
    /// Start date.
    #[serde(default)]
    pub start_date: Option<String>,
    /// Due date, optionally with a time.
    #[serde(default)]
    pub due: Option<String>,
    /// Notes as Vellum Markdown.
    #[serde(default)]
    pub notes: Option<String>,
    /// Assignee handle; the user when absent.
    #[serde(default)]
    pub assignee: Option<String>,
}

/// A collection query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionQuery {
    /// Collection id.
    pub collection: String,
    /// Saved view name.
    #[serde(default)]
    pub view: Option<String>,
    /// Filters: `{property, op, value}` objects.
    #[serde(default)]
    pub filters: Vec<Value>,
    /// Sorts: `{property, direction}` objects.
    #[serde(default)]
    pub sorts: Vec<Value>,
    /// Maximum rows.
    pub limit: u32,
    /// Opaque cursor.
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Rows with computed values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowPage {
    /// Property names in schema order.
    pub columns: Vec<String>,
    /// Rows: `id` plus property values, formulas and rollups computed.
    pub rows: Vec<Value>,
    /// Cursor for the next page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Rows to create or update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RowUpsert {
    /// Collection id.
    pub collection: String,
    /// Rows: with `id` to update, without to create.
    pub rows: Vec<Value>,
}

/// An export request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportRequest {
    /// Document or book id.
    pub target: String,
    /// `pdf`, `docx` or `md`.
    pub format: String,
    /// For a book: a section id to export that subtree only.
    #[serde(default)]
    pub section: Option<String>,
}

/// A finished export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportArtifact {
    /// Download URL (signed, short-lived).
    pub url: String,
    /// File name.
    pub name: String,
    /// MIME type.
    pub mime_type: String,
    /// Size in bytes.
    pub size: u64,
    /// The file itself, base64, when small enough to hand over inline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// One change since a point in time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// `vellum://` URI of the changed object.
    pub uri: String,
    /// Title or issue key and title.
    pub title: String,
    /// created, edited, moved, deleted, `state_changed`, commented, …
    pub action: String,
    /// Who made the change.
    pub actor: String,
    /// When, RFC 3339.
    pub at: String,
}

/// A page of changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangePage {
    /// Changes, oldest first.
    pub changes: Vec<Change>,
    /// Cursor for the next page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// A readable object listed by `resources/list`, the sitemap and
/// `llms.txt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    /// `vellum://` URI.
    pub uri: String,
    /// Title.
    pub title: String,
    /// One-line description, when known.
    #[serde(default)]
    pub description: Option<String>,
    /// App URL (add `?format=md` for Markdown).
    pub url: String,
    /// Space slug or name, for grouping.
    pub space: String,
}

/// A page of listings.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ListingPage {
    /// Entries.
    pub items: Vec<Listing>,
    /// Cursor for the next page.
    pub next_cursor: Option<String>,
}

/// A page template a model can start a draft from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    /// The name to pass to `get_template` (lowercase, hyphenated).
    pub name: String,
    /// The template page's title.
    pub title: String,
    /// `vellum://template/<name>`.
    pub uri: String,
}

/// Facts about the deployment, for `llms.txt`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteInfo {
    /// Workspace name, such as `Northstar`.
    pub name: String,
    /// One-paragraph summary.
    pub summary: String,
    /// Public base URL.
    pub base_url: String,
}

/// Port: the workspace as the MCP server sees it.
#[async_trait]
pub trait Workspace: Send + Sync {
    /// Hybrid, permission-filtered, block-level search.
    async fn search(
        &self,
        actor: &Actor,
        query: SearchQuery,
    ) -> Result<Vec<SearchHit>, WorkspaceError>;
    /// A page as Markdown with front matter.
    async fn get_doc(&self, actor: &Actor, id: &str) -> Result<Document, WorkspaceError>;
    /// A page found by its app path, for `?format=md`.
    async fn get_doc_by_path(&self, actor: &Actor, path: &str) -> Result<Document, WorkspaceError>;
    /// Selected blocks of a page.
    async fn get_blocks(
        &self,
        actor: &Actor,
        doc_id: &str,
        selector: BlockSelector,
    ) -> Result<Vec<Block>, WorkspaceError>;
    /// The heading tree of a page, or the section tree of a book.
    async fn get_outline(
        &self,
        actor: &Actor,
        id: &str,
    ) -> Result<Vec<OutlineNode>, WorkspaceError>;
    /// A book: `book.yaml`, `SUMMARY.md` and numbering, as Markdown.
    async fn get_book(&self, actor: &Actor, id: &str) -> Result<Document, WorkspaceError>;
    /// A space's tree as a Markdown nested list of links.
    async fn space_tree(&self, actor: &Actor, slug: &str) -> Result<Document, WorkspaceError>;
    /// Writes a new page.
    async fn create_page(
        &self,
        actor: &Actor,
        page: NewPage,
        mode: WriteMode,
        by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError>;
    /// Applies or proposes block edits.
    async fn edit_page(
        &self,
        actor: &Actor,
        edit: EditProposal,
        mode: WriteMode,
        by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError>;
    /// Reads an issue by key.
    async fn get_issue(&self, actor: &Actor, key: &str) -> Result<Issue, WorkspaceError>;
    /// Files an issue; model-filed issues land in the team's triage.
    async fn create_issue(
        &self,
        actor: &Actor,
        issue: NewIssue,
        by: Attribution,
    ) -> Result<Issue, WorkspaceError>;
    /// Updates an issue.
    async fn update_issue(
        &self,
        actor: &Actor,
        update: IssueUpdate,
        by: Attribution,
    ) -> Result<Issue, WorkspaceError>;
    /// Adds a comment.
    async fn add_comment(
        &self,
        actor: &Actor,
        comment: NewComment,
        by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError>;
    /// Creates a task in My Work.
    async fn create_task(
        &self,
        actor: &Actor,
        task: NewTask,
        by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError>;
    /// Queries a collection, computing formulas and rollups.
    async fn query_collection(
        &self,
        actor: &Actor,
        query: CollectionQuery,
    ) -> Result<RowPage, WorkspaceError>;
    /// A collection view rendered as a Markdown table, for resources.
    async fn get_collection(
        &self,
        actor: &Actor,
        id: &str,
        view: Option<&str>,
    ) -> Result<Document, WorkspaceError>;
    /// Creates or updates rows.
    async fn upsert_rows(
        &self,
        actor: &Actor,
        upsert: RowUpsert,
        mode: WriteMode,
        by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError>;
    /// Exports a page or book.
    async fn export(
        &self,
        actor: &Actor,
        request: ExportRequest,
    ) -> Result<ExportArtifact, WorkspaceError>;
    /// What changed since a timestamp.
    async fn list_changes(
        &self,
        actor: &Actor,
        since: &str,
        limit: u32,
        cursor: Option<String>,
    ) -> Result<ChangePage, WorkspaceError>;
    /// Spaces, books and collections the actor can read, for listings.
    async fn list_readable(
        &self,
        actor: &Actor,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<ListingPage, WorkspaceError>;
    /// A named page template's Markdown (for the `write-prd` prompt and the
    /// `get_template` tool).
    async fn get_template(&self, actor: &Actor, name: &str) -> Result<String, WorkspaceError>;
    /// The page templates the actor can use, by name and title.
    async fn list_templates(&self, _actor: &Actor) -> Result<Vec<Template>, WorkspaceError> {
        Ok(Vec::new())
    }
    /// Deployment facts for `llms.txt`.
    async fn site_info(&self) -> Result<SiteInfo, WorkspaceError>;
}

/// One audited MCP call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    /// When, RFC 3339 UTC.
    pub at: String,
    /// The request id returned in headers.
    pub request_id: String,
    /// W3C trace id, when the call was traced.
    pub trace_id: Option<String>,
    /// The user id, or `anonymous`.
    pub user_id: String,
    /// The client id.
    pub client_id: String,
    /// The token id.
    pub token_id: String,
    /// JSON-RPC method.
    pub method: String,
    /// Tool, prompt or resource name.
    pub target: Option<String>,
    /// Ids touched by the call (read or written).
    pub touched: Vec<String>,
    /// `ok`, `tool_error`, `denied`, `rate_limited` or `error`.
    pub outcome: String,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// Port: the append-only audit log (audit/admin service).
#[async_trait]
pub trait AuditSink: Send + Sync {
    /// Appends one event. Failures are reported but never fail the call.
    ///
    /// # Errors
    ///
    /// When the event could not be stored.
    async fn record(&self, event: AuditEvent) -> Result<(), WorkspaceError>;
}
