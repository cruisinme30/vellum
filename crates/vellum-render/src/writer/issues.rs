// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `::: issues`: a live list or board of issues embedded in a page.

use vellum_format::Container;

use super::Writer;
use super::inline::safe_url;
use crate::html::{self, push_attr};
use crate::resolve::{IssueCard, IssueGroup, IssueLayout, IssueView, IssuesAt};

impl Writer<'_> {
    pub(super) fn issues(&mut self, c: &Container) {
        let index = self.issue_blocks;
        self.issue_blocks += 1;
        let block_id = c.id.as_ref().map(ToString::to_string);
        let at = IssuesAt {
            index,
            block_id: block_id.as_deref(),
        };
        let view = match self.resolver().issues(at) {
            Some(Ok(view)) => Some(view),
            Some(Err(reason)) => {
                self.warn(format!("issues block {}: {reason}", index + 1));
                None
            }
            None => None,
        };
        let layout = match view.as_ref().map(|v| v.layout) {
            Some(IssueLayout::Board) => "board",
            _ => "list",
        };
        self.out.push_str("<section");
        self.attrs(&format!("vl-issues vl-issues-{layout}"), &c.attrs);
        push_attr(&mut self.out, "data-name", "issues");
        push_attr(&mut self.out, "data-index", &index.to_string());
        push_attr(
            &mut self.out,
            "aria-label",
            view.as_ref()
                .and_then(|v| v.title.as_deref())
                .unwrap_or("Issues"),
        );
        self.out.push('>');
        match view {
            Some(view) => self.issue_view(&view),
            None => self.out.push_str(
                "<p class=\"vl-live-placeholder\">A live view of issues; it fills in when the page opens.</p>",
            ),
        }
        self.out.push_str("</section>");
    }

    fn issue_view(&mut self, view: &IssueView) {
        if let Some(title) = &view.title {
            self.out.push_str("<p class=\"vl-issues-title\"><strong>");
            html::text(&mut self.out, title);
            self.out.push_str("</strong></p>");
        }
        let total: usize = view.groups.iter().map(|g| g.issues.len()).sum();
        if total == 0 && view.layout == IssueLayout::List {
            self.out
                .push_str("<p class=\"vl-issues-empty\">No issues match this view.</p>");
        } else if view.layout == IssueLayout::Board {
            self.out.push_str("<div class=\"vl-issues-board\">");
            for g in &view.groups {
                self.out.push_str("<section class=\"vl-issues-col\"");
                push_attr(&mut self.out, "aria-label", group_title(g));
                self.out.push('>');
                self.group_heading(g);
                self.issue_list(&g.issues);
                self.out.push_str("</section>");
            }
            self.out.push_str("</div>");
        } else {
            for g in view.groups.iter().filter(|g| !g.issues.is_empty()) {
                if !g.title.is_empty() {
                    self.group_heading(g);
                }
                self.issue_list(&g.issues);
            }
        }
        if view.more > 0 {
            self.out.push_str("<p class=\"vl-issues-more\">");
            html::text(
                &mut self.out,
                &format!(
                    "{} more issue{} not shown.",
                    view.more,
                    if view.more == 1 { "" } else { "s" }
                ),
            );
            self.out.push_str("</p>");
        }
    }

    fn group_heading(&mut self, g: &IssueGroup) {
        self.out.push_str("<p class=\"vl-issues-group\"><strong>");
        html::text(&mut self.out, group_title(g));
        self.out.push_str("</strong> <span class=\"vl-count\">");
        html::text(&mut self.out, &g.issues.len().to_string());
        self.out.push_str("</span></p>");
    }

    fn issue_list(&mut self, issues: &[IssueCard]) {
        self.out.push_str("<ul class=\"vl-issue-list\">");
        for i in issues {
            self.out
                .push_str("<li class=\"vl-issue-row\"><a class=\"vl-issue\"");
            if let Some(href) = safe_url(self, &i.href) {
                push_attr(&mut self.out, "href", &href);
            }
            push_attr(&mut self.out, "data-issue", &i.key);
            self.out.push('>');
            html::text(&mut self.out, &i.key);
            self.out.push_str("</a> <span class=\"vl-issue-title\">");
            html::text(&mut self.out, &i.title);
            self.out.push_str("</span>");
            for (class, value) in [
                (
                    "vl-issue-state",
                    Some(i.state.as_str()).filter(|s| !s.is_empty()),
                ),
                ("vl-issue-priority", i.priority.as_deref()),
                ("vl-issue-assignee", i.assignee.as_deref()),
            ] {
                if let Some(v) = value {
                    self.out.push_str(" <span class=\"");
                    self.out.push_str(class);
                    self.out.push_str("\">");
                    html::text(&mut self.out, v);
                    self.out.push_str("</span>");
                }
            }
            self.out.push_str("</li>");
        }
        self.out.push_str("</ul>");
    }
}

fn group_title(g: &IssueGroup) -> &str {
    if g.title.is_empty() {
        "No value"
    } else {
        &g.title
    }
}
