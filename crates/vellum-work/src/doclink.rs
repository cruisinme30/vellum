// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Issues and documents linked both ways (R-WK-7, D18, D131).
//!
//! - **Issue from a doc.** Selecting text and choosing "Create issue" files
//!   an issue whose `links` hold the source block (`kind: source`), and
//!   writes the new key into that block as a live chip.
//! - **Back-links.** Any issue key written in a document is a chip. The
//!   service indexes [`WorkGraph::backlinks`] per document so an issue can
//!   list every block that mentions it.
//! - **Doc from an issue.** A new document starts from [`doc_draft`] and
//!   the issue links it (`kind: doc`).
//! - **Embedded views.** A project page embeds a list or board through an
//!   `::: issues` block whose YAML body is an [`IssueEmbed`].

use serde::{Deserialize, Serialize};
use vellum_kernel::DocId;

use crate::comment::BlockRef;
use crate::error::{Result, WorkError};
use crate::filter::Filter;
use crate::graph::{Changes, Ctx, IssuePatch, NewIssue, WorkGraph};
use crate::id::{IssueId, UserRef, ViewId};
use crate::issue::{Issue, IssueKey, IssueLink};
use crate::mention;
use crate::view::{self, GroupBy, Layout, OrderBy, ShowClosed, View, Visibility};

/// The link kind for the block an issue was created from.
pub const SOURCE: &str = "source";

/// The link kind for a document made from, or attached to, an issue.
pub const DOC: &str = "doc";

/// The largest number of issues an embedded view shows.
pub const MAX_EMBED: usize = 200;

/// The issue link to the block an issue was created from.
#[must_use]
pub fn source_link(at: &BlockRef, quote: Option<&str>) -> IssueLink {
    IssueLink {
        url: at.url(),
        title: quote.map(|q| q.trim().chars().take(200).collect()),
        kind: Some(SOURCE.to_owned()),
    }
}

/// The issue link to a document.
#[must_use]
pub fn doc_link(doc: DocId, title: &str) -> IssueLink {
    IssueLink {
        url: format!("vellum://doc/{doc}"),
        title: Some(title.to_owned()),
        kind: Some(DOC.to_owned()),
    }
}

/// The block an issue was created from, if any.
#[must_use]
pub fn source_of(issue: &Issue) -> Option<BlockRef> {
    issue
        .links
        .iter()
        .filter(|l| l.kind.as_deref() == Some(SOURCE))
        .find_map(|l| BlockRef::from_url(&l.url))
}

/// Writes `key` into a block as a chip, just before its block ID.
///
/// The document is returned unchanged if the block already shows the key.
///
/// # Errors
///
/// Returns [`WorkError::Invalid`] if no block has this ID.
pub fn insert_chip(markdown: &str, block: &str, key: &IssueKey) -> Result<String> {
    let found = mention::blocks(markdown)
        .into_iter()
        .find(|b| b.id == Some(block))
        .ok_or_else(|| WorkError::Invalid {
            what: "block ID",
            value: block.to_owned(),
            reason: "is not in the document",
        })?;
    if mention::issue_keys(found.text).contains(key) {
        return Ok(markdown.to_owned());
    }
    let marker = format!(" ^{block}");
    let at = found.range.start
        + found
            .text
            .rfind(&marker)
            .ok_or_else(|| WorkError::Invalid {
                what: "block ID",
                value: block.to_owned(),
                reason: "is not at the end of its block",
            })?;
    let mut out = String::with_capacity(markdown.len() + 16);
    out.push_str(markdown.get(..at).unwrap_or_default());
    out.push(' ');
    out.push_str(&key.to_string());
    out.push_str(markdown.get(at..).unwrap_or_default());
    Ok(out)
}

/// One issue chip in a document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackLink {
    /// The issue.
    pub issue: IssueId,
    /// The key as written, which may be an old alias.
    pub written: IssueKey,
    /// The document.
    pub doc: DocId,
    /// The block holding the chip; `None` if the block has no ID yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<String>,
}

/// A new document made from an issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocDraft {
    /// The document title.
    pub title: String,
    /// The body: a chip back to the issue, then the issue's description.
    pub body: String,
}

/// The starting point for "Create doc from issue".
#[must_use]
pub fn doc_draft(issue: &Issue) -> DocDraft {
    let mut body = format!("Created from {}.\n", issue.key());
    let description = issue.body.trim();
    if !description.is_empty() {
        body.push('\n');
        body.push_str(description);
        body.push('\n');
    }
    DocDraft {
        title: issue.title.clone(),
        body,
    }
}

/// An issue view embedded in a page, read from an `::: issues` block.
///
/// It either names a saved view, optionally overriding its layout, or
/// gives a filter inline. Whatever the page says, the reader only sees
/// issues they may see; the service filters the result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IssueEmbed {
    /// A saved view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewId>,
    /// An inline filter, used when no view is named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,
    /// List or board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    /// Grouping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_by: Option<GroupBy>,
    /// Ordering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order_by: Option<OrderBy>,
    /// Which closed issues to show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_closed: Option<ShowClosed>,
    /// At most this many issues (default and ceiling [`MAX_EMBED`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

impl IssueEmbed {
    /// Reads the YAML body of an `::: issues` block.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] for unknown keys, a bad filter, or
    /// neither a view nor a filter.
    pub fn from_yaml(text: &str) -> Result<Self> {
        let embed: Self = serde_saphyr::from_str(text).map_err(|e| WorkError::Invalid {
            what: "issue embed",
            value: e.to_string(),
            reason: "is not a valid issues block",
        })?;
        if embed.view.is_none() && embed.filter.is_none() {
            return Err(WorkError::Invalid {
                what: "issue embed",
                value: text.trim().to_owned(),
                reason: "needs a view or a filter",
            });
        }
        if let Some(f) = &embed.filter {
            f.check().map_err(|_| WorkError::Invalid {
                what: "issue embed",
                value: text.trim().to_owned(),
                reason: "has an invalid filter",
            })?;
        }
        Ok(embed)
    }

    /// The view to run: the saved view with this block's overrides, or an
    /// unsaved view from the inline filter.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] if the block names a view but
    /// `saved` is not that view.
    pub fn to_view(&self, saved: Option<&View>, viewer: &UserRef) -> Result<View> {
        let mut v = match (self.view, saved) {
            (Some(id), Some(s)) if s.id == id => s.clone(),
            (Some(id), _) => {
                return Err(WorkError::Invalid {
                    what: "issue embed",
                    value: id.to_string(),
                    reason: "names a view that was not found",
                });
            }
            (None, _) => View {
                id: ViewId::generate(),
                name: "Embedded issues".to_owned(),
                owner: viewer.clone(),
                team: None,
                visibility: Visibility::Private,
                layout: Layout::default(),
                filter: self.filter.clone().unwrap_or_default(),
                group_by: GroupBy::None,
                order_by: OrderBy::default(),
                show_closed: ShowClosed::default(),
                show_sub_issues: true,
            },
        };
        if let Some(f) = &self.filter
            && self.view.is_some()
        {
            v.filter = Filter::And {
                and: vec![v.filter.clone(), f.clone()],
            };
        }
        if let Some(l) = self.layout {
            v.layout = l;
        }
        if let Some(g) = self.group_by {
            v.group_by = g;
        }
        if let Some(o) = self.order_by {
            v.order_by = o;
        }
        if let Some(s) = self.show_closed {
            v.show_closed = s;
        }
        Ok(v)
    }
}

impl WorkGraph {
    /// Creates an issue from a block of a document and writes its chip
    /// into that block (R-WK-7).
    ///
    /// Returns the issue's changes and the document text with the chip.
    /// The block is checked before the issue is numbered, so a bad block
    /// changes nothing.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] if the block is not in `markdown`,
    /// or any [`WorkGraph::create_issue`] error.
    pub fn issue_from_block(
        &mut self,
        mut draft: NewIssue,
        at: &BlockRef,
        quote: Option<&str>,
        markdown: &str,
        ctx: &Ctx,
    ) -> Result<(Changes, String)> {
        if !mention::blocks(markdown)
            .iter()
            .any(|b| b.id == Some(at.block.as_str()))
        {
            return Err(WorkError::Invalid {
                what: "block ID",
                value: at.block.clone(),
                reason: "is not in the document",
            });
        }
        draft.links.push(source_link(at, quote));
        let id = draft.id;
        let changes = self.create_issue(draft, ctx)?;
        let key = self
            .issue(&id)
            .map(Issue::key)
            .ok_or_else(|| WorkError::UnknownIssue(id.to_string()))?;
        let doc = insert_chip(markdown, &at.block, &key)?;
        Ok((changes, doc))
    }

    /// Links a document to an issue, such as one made with [`doc_draft`].
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownIssue`] for an unknown issue.
    pub fn link_doc(
        &mut self,
        issue: &IssueId,
        doc: DocId,
        title: &str,
        ctx: &Ctx,
    ) -> Result<Changes> {
        let patch = IssuePatch {
            add_links: vec![doc_link(doc, title)],
            ..IssuePatch::default()
        };
        self.update_issue(issue, patch, ctx)
    }

    /// Every issue chip in a document, in order, including keys that are
    /// now aliases. Keys of unknown issues are not chips.
    #[must_use]
    pub fn backlinks(&self, doc: DocId, markdown: &str) -> Vec<BackLink> {
        let mut out = Vec::new();
        for block in mention::blocks(markdown) {
            for key in mention::issue_keys(block.text) {
                if let Some(issue) = self.resolve(&key.to_string()) {
                    out.push(BackLink {
                        issue: issue.id,
                        written: key,
                        doc,
                        block: block.id.map(str::to_owned),
                    });
                }
            }
        }
        out
    }

    /// Runs an embedded view for `viewer`, capped at the embed's limit.
    ///
    /// # Errors
    ///
    /// Returns the [`IssueEmbed::to_view`] error.
    pub fn embed(
        &self,
        embed: &IssueEmbed,
        saved: Option<&View>,
        viewer: &UserRef,
        ctx: &Ctx,
    ) -> Result<Vec<view::Group>> {
        let v = embed.to_view(saved, viewer)?;
        let mut left = embed.limit.unwrap_or(MAX_EMBED).min(MAX_EMBED);
        let mut groups = view::run(&v, self, ctx, Some(viewer));
        for g in &mut groups {
            g.issues.truncate(left);
            left -= g.issues.len();
        }
        Ok(groups)
    }
}
