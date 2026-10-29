// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Live `::: issues` views inside documents.
//!
//! A page embeds a list or board with an `::: issues` container whose
//! YAML body is an [`IssueEmbed`]. The renderer cannot call Work, so the
//! server finds the blocks here with [`issue_blocks`], runs each with
//! [`WorkGraph::run_embed`], and hands the renderer plain rows to draw.
//! Blocks are matched by block ID when they have one, else by position.

use crate::doclink::{IssueEmbed, MAX_EMBED};
use crate::error::Result;
use crate::graph::{Ctx, WorkGraph};
use crate::id::{ProjectId, UserRef};
use crate::scale::Priority;
use crate::view::{self, GroupBy, Layout, View};
use crate::workflow::StatusKey;

/// The raw body of one `::: issues` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueBlock {
    /// The block ID from the opening line (`blk-…`), if any.
    pub block_id: Option<String>,
    /// The YAML between the opening and closing lines, indentation kept.
    pub body: String,
}

/// Finds the `::: issues` blocks of a document, in document order.
///
/// It follows the format's container rules: an opening line is three or
/// more colons and a name, optionally ending in `^blk-…`; a line of only
/// colons closes the innermost open container; fenced code is skipped.
/// Blocks inside other containers count, in the order they open.
#[must_use]
pub fn issue_blocks(markdown: &str) -> Vec<IssueBlock> {
    let mut out: Vec<IssueBlock> = Vec::new();
    // For each open container: the index in `out` if it is an issues block.
    let mut open: Vec<Option<usize>> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in markdown.lines() {
        let bare = line.trim_end();
        let lead = bare.len() - bare.trim_start().len();
        let text = bare.trim_start();
        if let Some((ch, len)) = fence {
            let run = text.len() - text.trim_start_matches(ch).len();
            if lead < 4 && run >= len && text.trim_start_matches(ch).trim().is_empty() {
                fence = None;
            }
            append(&mut out, &open, line);
            continue;
        }
        if lead < 4
            && let Some(ch) = ['`', '~'].into_iter().find(|c| text.starts_with(*c))
        {
            let run = text.len() - text.trim_start_matches(ch).len();
            if run >= 3 {
                fence = Some((ch, run));
                append(&mut out, &open, line);
                continue;
            }
        }
        let colons = bare.len() - bare.trim_start_matches(':').len();
        if colons >= 3 {
            let rest = bare.get(colons..).unwrap_or_default().trim();
            if rest.is_empty() {
                let _ = open.pop();
                continue;
            }
            if let Some((name, id)) = opener(rest) {
                if name == "issues" {
                    out.push(IssueBlock {
                        block_id: id,
                        body: String::new(),
                    });
                    open.push(Some(out.len() - 1));
                } else {
                    open.push(None);
                }
                continue;
            }
        }
        append(&mut out, &open, line);
    }
    out
}

/// Adds a line to the innermost open container if it is an issues block.
fn append(out: &mut [IssueBlock], open: &[Option<usize>], line: &str) {
    if let Some(Some(i)) = open.last()
        && let Some(block) = out.get_mut(*i)
    {
        block.body.push_str(line);
        block.body.push('\n');
    }
}

/// The name and block ID of an opening line, after its colons.
fn opener(rest: &str) -> Option<(&str, Option<String>)> {
    let head = rest.split('{').next().unwrap_or_default().trim_end();
    let (head, id) = match head.rsplit_once(char::is_whitespace) {
        Some((before, last)) if last.starts_with("^blk-") => {
            (before.trim_end(), last.strip_prefix('^').map(str::to_owned))
        }
        _ if head.starts_with("^blk-") => ("", head.strip_prefix('^').map(str::to_owned)),
        _ => (head, None),
    };
    let name = head.split_whitespace().next().unwrap_or_default();
    let valid = name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    (valid || (name.is_empty() && (id.is_some() || rest.contains('{')))).then_some((name, id))
}

/// One group of an embedded view, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedGroup {
    /// What a reader sees: a status, priority or project name, a handle
    /// or a label; empty when the view is not grouped.
    pub title: String,
    /// The issues, in order.
    pub issues: Vec<crate::id::IssueId>,
}

/// An embedded view after it ran for a reader.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbedRun {
    /// List or board.
    pub layout: Layout,
    /// The groups, in order.
    pub groups: Vec<EmbedGroup>,
    /// Issues that matched but were cut by the limit.
    pub more: usize,
}

impl WorkGraph {
    /// Runs an embedded view for `viewer`, with display titles and a
    /// count of the issues the limit cut.
    ///
    /// # Errors
    ///
    /// Returns [`crate::WorkError::Invalid`] if the block names a view
    /// that `saved` is not.
    pub fn run_embed(
        &self,
        embed: &IssueEmbed,
        saved: Option<&View>,
        viewer: &UserRef,
        ctx: &Ctx,
    ) -> Result<EmbedRun> {
        let v = embed.to_view(saved, viewer)?;
        let mut left = embed.limit.unwrap_or(MAX_EMBED).min(MAX_EMBED);
        let mut more = 0;
        let groups = view::run(&v, self, ctx, Some(viewer))
            .into_iter()
            .map(|g| {
                let title = self.group_title(v.group_by, g.key.as_deref());
                let mut issues = g.issues;
                let keep = issues.len().min(left);
                more += issues.len() - keep;
                issues.truncate(keep);
                left -= keep;
                EmbedGroup { title, issues }
            })
            .collect();
        Ok(EmbedRun {
            layout: v.layout,
            groups,
            more,
        })
    }

    /// The name a reader sees for a group's value.
    fn group_title(&self, by: GroupBy, key: Option<&str>) -> String {
        let Some(key) = key else {
            return match by {
                GroupBy::None => String::new(),
                GroupBy::Assignee => "No assignee".to_owned(),
                GroupBy::Project => "No project".to_owned(),
                GroupBy::Cycle => "No cycle".to_owned(),
                GroupBy::Label => "No label".to_owned(),
                GroupBy::Priority => "No priority".to_owned(),
                GroupBy::Status | GroupBy::Team => "None".to_owned(),
            };
        };
        let found = match by {
            GroupBy::Status => StatusKey::parse(key).ok().and_then(|s| {
                self.teams()
                    .find_map(|t| t.workflow.get(&s).map(|st| st.name.clone()))
            }),
            GroupBy::Priority => Priority::from_name(key).map(|p| {
                let name = p.name();
                let mut chars = name.chars();
                chars.next().map_or_else(String::new, |c| {
                    c.to_ascii_uppercase().to_string() + chars.as_str()
                })
            }),
            GroupBy::Project => key
                .parse::<ProjectId>()
                .ok()
                .and_then(|id| self.project(&id).map(|p| p.title.clone())),
            GroupBy::Assignee => Some(format!("@{key}")),
            GroupBy::Team => self
                .teams()
                .find(|t| t.key.to_string() == key)
                .map(|t| t.name.clone()),
            GroupBy::Cycle | GroupBy::Label | GroupBy::None => None,
        };
        found.unwrap_or_else(|| key.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_issue_blocks_in_order_with_ids_and_indentation() {
        let doc = "\
# Plan

```
::: issues
not: this
:::
```

:::: callout
::: issues ^blk-aaaaaaaa
filter:
  field: team
  op: is
  values: [ENG]
:::
::::

::: issues
view: 01JC0000000000000000000V01
:::
";
        let blocks = issue_blocks(doc);
        assert_eq!(blocks.len(), 2, "{blocks:?}");
        assert_eq!(blocks[0].block_id.as_deref(), Some("blk-aaaaaaaa"));
        assert!(blocks[0].body.contains("\n  op: is\n"));
        assert_eq!(blocks[1].block_id, None);
        assert_eq!(blocks[1].body, "view: 01JC0000000000000000000V01\n");
    }
}
