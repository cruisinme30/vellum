// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Live `::: issues` views for the page renderer.
//!
//! The renderer cannot call Work, so the server asks here for every
//! `::: issues` block of a page and passes the rows on. Each block gets
//! its own result, so one bad block shows a warning without hiding the
//! others.

use vellum_work::embed::{EmbedRun, issue_blocks};
use vellum_work::view::Layout;
use vellum_work::{IssueEmbed, UserRef, WorkGraph};

use super::WorkService;

/// One issue row in an embedded view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedIssue {
    /// Its identifier, such as `ENG-142`.
    pub key: String,
    /// Where the web app shows it.
    pub href: String,
    /// Its title.
    pub title: String,
    /// Its status, as named in the team's workflow.
    pub state: String,
    /// Its assignee's handle.
    pub assignee: Option<String>,
    /// Its priority, 0 (none) to 4 (low).
    pub priority: u8,
}

/// One group of an embedded view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedGroup {
    /// What a reader sees; empty when the view is not grouped.
    pub title: String,
    /// The rows, in order.
    pub issues: Vec<EmbedIssue>,
}

/// An embedded view, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedView {
    /// Whether to draw columns instead of rows.
    pub board: bool,
    /// The groups, in order.
    pub groups: Vec<EmbedGroup>,
    /// Issues that matched but were cut by the block's limit.
    pub more: usize,
}

/// One `::: issues` block of a page and what it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedResult {
    /// The block's ID (`blk-…`), when the opening line has one.
    pub block_id: Option<String>,
    /// The view, or why it cannot be shown.
    pub view: Result<EmbedView, String>,
}

impl EmbedView {
    fn of(run: EmbedRun, graph: &WorkGraph) -> Self {
        let groups = run
            .groups
            .into_iter()
            .map(|g| EmbedGroup {
                title: g.title,
                issues: g
                    .issues
                    .iter()
                    .filter_map(|id| graph.issue(id))
                    .map(|i| {
                        let key = i.key().to_string();
                        EmbedIssue {
                            href: format!("/work/issues/{key}"),
                            key,
                            title: i.title.clone(),
                            state: graph
                                .team(&i.team)
                                .and_then(|t| t.workflow.get(&i.state))
                                .map_or_else(|| i.state.to_string(), |s| s.name.clone()),
                            assignee: i.assignee.as_ref().map(UserRef::to_string),
                            priority: i.priority.number(),
                        }
                    })
                    .collect(),
            })
            .collect();
        Self {
            board: run.layout == Layout::Board,
            groups,
            more: run.more,
        }
    }
}

impl WorkService {
    /// Runs every `::: issues` block of `markdown` for `viewer`, in
    /// document order.
    ///
    /// Saved views are not stored by the service yet, so a block that
    /// names one reports that instead of a view.
    pub async fn issue_embeds(&self, markdown: &str, viewer: &UserRef) -> Vec<EmbedResult> {
        let blocks = issue_blocks(markdown);
        if blocks.is_empty() {
            return Vec::new();
        }
        let now = self.clock.now();
        let ctx = vellum_work::Ctx {
            now,
            today: now.date(),
            actor: Some(viewer.clone()),
        };
        let st = self.read_state().await;
        blocks
            .into_iter()
            .map(|b| EmbedResult {
                view: IssueEmbed::from_yaml(&b.body)
                    .and_then(|e| st.graph.run_embed(&e, None, viewer, &ctx))
                    .map(|run| EmbedView::of(run, &st.graph))
                    .map_err(|e| e.to_string()),
                block_id: b.block_id,
            })
            .collect()
    }
}
