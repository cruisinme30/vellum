// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The workspace as the assistant sees it: permission-filtered search and
//! reads as the signed-in user, and draft pages for review.
//!
//! The binary implements this port over the MCP server's workspace adapter,
//! so the assistant sees exactly what that user's MCP client would see.

use async_trait::async_trait;
use vellum_contracts::principal::Principal;
use vellum_kernel::Error;

/// One block-level search hit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// The page's id.
    pub doc_id: String,
    /// The page's title.
    pub title: String,
    /// Headings above the block.
    pub heading_path: Vec<String>,
    /// The block's text, as Markdown.
    pub snippet: String,
    /// A link that opens the block in the app.
    pub url: String,
}

/// A page as Markdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    /// The page's id.
    pub id: String,
    /// The page's title.
    pub title: String,
    /// The page file: front matter and body.
    pub markdown: String,
    /// A link that opens the page in the app.
    pub url: String,
    /// The space's slug.
    pub space: String,
}

/// A page the model wrote, to land as a draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftPage {
    /// A page id, or `space:<slug>`.
    pub parent: String,
    /// The title.
    pub title: String,
    /// The body as Vellum Markdown, without front matter.
    pub markdown: String,
    /// The template page it follows.
    pub template: Option<String>,
}

/// A page that was created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    /// The new page's id.
    pub id: String,
    /// A link that opens it.
    pub url: String,
}

/// Port: what the assistant reads and writes, always as `who`.
#[async_trait]
pub trait Sources: Send + Sync {
    /// Hybrid, permission-filtered, block-level search.
    async fn search(
        &self,
        who: &Principal,
        query: &str,
        space: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Hit>, Error>;

    /// A page `who` may read.
    async fn page(&self, who: &Principal, id: &str) -> Result<Page, Error>;

    /// Creates a draft page for `who` to review, attributed to the model.
    async fn create_draft(&self, who: &Principal, page: DraftPage) -> Result<Created, Error>;
}
