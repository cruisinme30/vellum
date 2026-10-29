// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The page renderer's `::: issues` lookup, answered from Work.
//!
//! Render asks for each `::: issues` block by position and block ID while
//! it writes a page, synchronously. [`WorkService::live_issues`] runs every
//! block of the page for the reader once, up front; [`LiveIssues`] then
//! answers render's lookups from that result. The server's page resolver
//! hands its `issues` calls to [`LiveIssues::view`], or uses
//! [`LiveIssues`] as the resolver when nothing else needs resolving.

use vellum_render::resolve::{IssueCard, IssueGroup, IssueLayout, IssueView, IssuesAt, Resolver};
use vellum_work::{Priority, UserRef};

use super::WorkService;
use super::embeds::{EmbedResult, EmbedView};

/// The `::: issues` views of one page, for one reader.
#[derive(Debug, Clone, Default)]
pub struct LiveIssues {
    blocks: Vec<EmbedResult>,
}

impl LiveIssues {
    /// The view for one block: by block ID when it has one, else by its
    /// position among the page's `::: issues` blocks.
    ///
    /// `None` means Work found no such block, so render shows its
    /// placeholder; `Err` explains a block that cannot be shown.
    #[must_use]
    pub fn view(&self, at: IssuesAt<'_>) -> Option<Result<IssueView, String>> {
        let found = at
            .block_id
            .and_then(|id| {
                self.blocks
                    .iter()
                    .find(|b| b.block_id.as_deref() == Some(id))
            })
            .or_else(|| self.blocks.get(at.index))?;
        Some(found.view.as_ref().map(to_render).map_err(Clone::clone))
    }
}

impl Resolver for LiveIssues {
    fn issues(&self, at: IssuesAt<'_>) -> Option<Result<IssueView, String>> {
        self.view(at)
    }
}

fn to_render(v: &EmbedView) -> IssueView {
    IssueView {
        title: None,
        layout: if v.board {
            IssueLayout::Board
        } else {
            IssueLayout::List
        },
        groups: v
            .groups
            .iter()
            .map(|g| IssueGroup {
                title: g.title.clone(),
                issues: g
                    .issues
                    .iter()
                    .map(|i| IssueCard {
                        key: i.key.clone(),
                        href: i.href.clone(),
                        title: i.title.clone(),
                        state: i.state.clone(),
                        assignee: i.assignee.as_ref().map(|a| format!("@{a}")),
                        priority: priority_name(i.priority),
                    })
                    .collect(),
            })
            .collect(),
        more: v.more,
    }
}

/// `Urgent`, `High`, `Normal` or `Low`; no priority shows nothing.
fn priority_name(n: u8) -> Option<String> {
    let p = Priority::from_number(n).filter(|p| *p != Priority::None)?;
    let name = p.name();
    let mut chars = name.chars();
    chars
        .next()
        .map(|c| c.to_ascii_uppercase().to_string() + chars.as_str())
}

impl WorkService {
    /// Runs every `::: issues` block of `markdown` for `viewer`, ready for
    /// the page renderer to look up.
    pub async fn live_issues(&self, markdown: &str, viewer: &UserRef) -> LiveIssues {
        LiveIssues {
            blocks: self.issue_embeds(markdown, viewer).await,
        }
    }
}
