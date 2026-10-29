// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Jira Cloud and Data Center issues, as the REST search API returns them
//! (`/rest/api/3/search/jql` or `/rest/api/2/search`), saved as JSON.
//!
//! Epics become projects and sub-tasks become sub-issues. Descriptions in
//! Atlassian Document Format (API v3) are converted to Markdown; v2 wiki
//! text is kept as it is. Story points come from the custom field the
//! admin names, because its ID differs per Jira site.

use std::fmt::Write as _;

use serde_json::Value;

use super::{Export, ImportError, ImportedIssue, Relation, Warning, timestamp};
use crate::inbound::text;
use crate::model::{Priority, StateCategory};

fn adf_inline(node: &Value, out: &mut String) {
    match text(node, "/type") {
        "text" => {
            let mut t = text(node, "/text").to_owned();
            if let Some(marks) = node.get("marks").and_then(Value::as_array) {
                for mark in marks {
                    t = match text(mark, "/type") {
                        "strong" => format!("**{t}**"),
                        "em" => format!("*{t}*"),
                        "strike" => format!("~~{t}~~"),
                        "code" => format!("`{t}`"),
                        "link" => format!("[{t}]({})", text(mark, "/attrs/href")),
                        _ => t,
                    };
                }
            }
            out.push_str(&t);
        }
        "hardBreak" => out.push('\n'),
        "mention" | "emoji" => out.push_str(text(node, "/attrs/text")),
        "inlineCard" => out.push_str(text(node, "/attrs/url")),
        _ => children(node, out, adf_inline),
    }
}

fn children(node: &Value, out: &mut String, each: fn(&Value, &mut String)) {
    if let Some(list) = node.get("content").and_then(Value::as_array) {
        for child in list {
            each(child, out);
        }
    }
}

fn adf_block(node: &Value, out: &mut String) {
    let inline = |node: &Value| {
        let mut s = String::new();
        children(node, &mut s, adf_inline);
        s
    };
    match text(node, "/type") {
        "paragraph" => {
            out.push_str(&inline(node));
            out.push_str("\n\n");
        }
        "heading" => {
            let level = node
                .pointer("/attrs/level")
                .and_then(Value::as_u64)
                .unwrap_or(2)
                .clamp(1, 6);
            let hashes = "#".repeat(usize::try_from(level).unwrap_or(2));
            let _ = write!(out, "{hashes} {}\n\n", inline(node));
        }
        "bulletList" | "orderedList" => {
            let ordered = text(node, "/type") == "orderedList";
            if let Some(items) = node.get("content").and_then(Value::as_array) {
                for (i, item) in items.iter().enumerate() {
                    let mut body = String::new();
                    children(item, &mut body, adf_block);
                    let marker = if ordered {
                        format!("{}.", i + 1)
                    } else {
                        "-".to_owned()
                    };
                    let _ = writeln!(out, "{marker} {}", body.trim().replace('\n', "\n  "));
                }
            }
            out.push('\n');
        }
        "codeBlock" => {
            let language = text(node, "/attrs/language");
            let _ = write!(out, "```{language}\n{}\n```\n\n", inline(node));
        }
        "blockquote" => {
            let mut body = String::new();
            children(node, &mut body, adf_block);
            for line in body.trim().lines() {
                let _ = writeln!(out, "> {line}");
            }
            out.push('\n');
        }
        "rule" => out.push_str("---\n\n"),
        "panel" => {
            let kind = match text(node, "/attrs/panelType") {
                "warning" => "warn",
                "error" => "crit",
                "success" => "good",
                "note" => "note",
                _ => "info",
            };
            let mut body = String::new();
            children(node, &mut body, adf_block);
            let _ = write!(out, "::: {kind}\n\n{}\n\n:::\n\n", body.trim());
        }
        _ => children(node, out, adf_block),
    }
}

/// Converts an Atlassian Document Format value to Markdown.
#[must_use]
pub fn adf_to_markdown(doc: &Value) -> String {
    let mut out = String::new();
    children(doc, &mut out, adf_block);
    out.trim().to_owned()
}

fn category(fields: &Value) -> Option<StateCategory> {
    let name = text(fields, "/status/name").to_ascii_lowercase();
    let resolution = text(fields, "/resolution/name").to_ascii_lowercase();
    match text(fields, "/status/statusCategory/key") {
        "done" => Some(match resolution.as_str() {
            "duplicate" => StateCategory::Duplicate,
            "won't do" | "won't fix" | "cannot reproduce" | "declined" | "incomplete" => {
                StateCategory::Canceled
            }
            _ => StateCategory::Completed,
        }),
        "indeterminate" => Some(StateCategory::Started),
        "new" if name == "backlog" => Some(StateCategory::Backlog),
        "new" => Some(StateCategory::Unstarted),
        _ => None,
    }
}

fn person(fields: &Value, field: &str) -> Option<String> {
    let person = fields.get(field)?;
    [text(person, "/emailAddress"), text(person, "/displayName")]
        .into_iter()
        .find(|v| !v.is_empty())
        .map(str::to_owned)
}

fn relations(fields: &Value) -> Vec<(Relation, String)> {
    let mut relations = Vec::new();
    for link in fields
        .get("issuelinks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = text(link, "/type/name").to_ascii_lowercase();
        let (outward, other) = match (
            text(link, "/outwardIssue/key"),
            text(link, "/inwardIssue/key"),
        ) {
            (out, _) if !out.is_empty() => (true, out),
            (_, inward) if !inward.is_empty() => (false, inward),
            _ => continue,
        };
        let relation = match (name.as_str(), outward) {
            ("blocks", true) => Relation::Blocks,
            ("blocks", false) => Relation::BlockedBy,
            ("duplicate", true) => Relation::DuplicateOf,
            ("duplicate", false) => continue,
            _ => Relation::Related,
        };
        relations.push((relation, other.to_owned()));
    }
    relations
}

/// Reads Jira search results. `estimate_field` names the story-points
/// field (such as `customfield_10016`), if any.
///
/// # Errors
///
/// [`ImportError::Unreadable`] if the JSON has no `issues` array.
pub fn read(json: &str, estimate_field: Option<&str>) -> Result<Export, ImportError> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| ImportError::Unreadable(e.to_string()))?;
    let issues = value
        .get("issues")
        .or(Some(&value))
        .and_then(Value::as_array)
        .ok_or_else(|| ImportError::Unreadable("expected an `issues` array".to_owned()))?;
    let mut export = Export::default();
    for issue in issues {
        let key = text(issue, "/key").to_owned();
        let Some(fields) = issue.get("fields") else {
            export.warnings.push(Warning {
                external_id: Some(key),
                message: "no fields; skipped".to_owned(),
            });
            continue;
        };
        let description = match fields.get("description") {
            Some(Value::String(wiki)) => wiki.clone(),
            Some(doc @ Value::Object(_)) => adf_to_markdown(doc),
            _ => String::new(),
        };
        let parent_is_epic =
            text(fields, "/parent/fields/issuetype/name").eq_ignore_ascii_case("epic");
        let (parent, project) = if parent_is_epic {
            (
                None,
                Some(text(fields, "/parent/fields/summary").to_owned()).filter(|s| !s.is_empty()),
            )
        } else {
            (
                Some(text(fields, "/parent/key").to_owned()).filter(|s| !s.is_empty()),
                None,
            )
        };
        let relations = relations(fields);
        let category = category(fields);
        let resolved = timestamp(text(fields, "/resolutiondate"));
        let mut labels: std::collections::BTreeSet<String> = fields
            .get("labels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let issue_type = text(fields, "/issuetype/name");
        if !issue_type.is_empty()
            && !issue_type.eq_ignore_ascii_case("task")
            && !issue_type.eq_ignore_ascii_case("sub-task")
        {
            let _ = labels.insert(issue_type.to_ascii_lowercase());
        }
        export.issues.push(ImportedIssue {
            external_id: key,
            title: text(fields, "/summary").to_owned(),
            description,
            state_name: text(fields, "/status/name").to_owned(),
            category,
            priority: Priority::parse(text(fields, "/priority/name")),
            estimate: estimate_field.and_then(|f| fields.get(f)).and_then(|v| {
                v.as_f64()
                    .map(|n| n.to_string())
                    .or_else(|| v.as_str().map(str::to_owned))
            }),
            labels,
            assignee: person(fields, "assignee"),
            creator: person(fields, "reporter").or_else(|| person(fields, "creator")),
            project,
            created_at: timestamp(text(fields, "/created")),
            completed_at: resolved.filter(|_| category == Some(StateCategory::Completed)),
            canceled_at: resolved.filter(|_| {
                matches!(
                    category,
                    Some(StateCategory::Canceled | StateCategory::Duplicate)
                )
            }),
            started_at: None,
            due: text(fields, "/duedate").parse().ok(),
            parent,
            relations,
        });
    }
    Ok(export)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adf_converts_to_markdown() {
        let doc = json!({"type": "doc", "version": 1, "content": [
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Steps"}]},
            {"type": "orderedList", "content": [
                {"type": "listItem", "content": [{"type": "paragraph", "content": [
                    {"type": "text", "text": "Open "}, {"type": "text", "text": "Export", "marks": [{"type": "strong"}]}]}]},
                {"type": "listItem", "content": [{"type": "paragraph", "content": [
                    {"type": "text", "text": "docs", "marks": [{"type": "link", "attrs": {"href": "https://northstar.dev"}}]}]}]}
            ]},
            {"type": "panel", "attrs": {"panelType": "warning"}, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Slow"}]}]},
            {"type": "codeBlock", "attrs": {"language": "rust"}, "content": [{"type": "text", "text": "fn main() {}"}]}
        ]});
        assert_eq!(
            adf_to_markdown(&doc),
            "## Steps\n\n1. Open **Export**\n2. [docs](https://northstar.dev)\n\n::: warn\n\nSlow\n\n:::\n\n```rust\nfn main() {}\n```"
        );
    }

    #[test]
    fn reads_search_results() -> Result<(), ImportError> {
        let json = json!({"issues": [
            {"key": "PROJ-7", "fields": {
                "summary": "Login loop", "description": "h2. Wiki text",
                "status": {"name": "Done", "statusCategory": {"key": "done"}},
                "resolution": {"name": "Won't Do"}, "resolutiondate": "2026-03-01T10:00:00.000+0000",
                "priority": {"name": "Highest"}, "labels": ["auth"], "issuetype": {"name": "Bug"},
                "assignee": {"emailAddress": "bo@northstar.dev", "displayName": "Bo"},
                "reporter": {"displayName": "Ada"}, "created": "2026-02-01T10:00:00.000+0000",
                "duedate": "2026-03-15", "customfield_10016": 5.0,
                "parent": {"key": "PROJ-1", "fields": {"summary": "Auth revamp", "issuetype": {"name": "Epic"}}},
                "issuelinks": [
                    {"type": {"name": "Blocks"}, "outwardIssue": {"key": "PROJ-9"}},
                    {"type": {"name": "Blocks"}, "inwardIssue": {"key": "PROJ-3"}},
                    {"type": {"name": "Duplicate"}, "inwardIssue": {"key": "PROJ-4"}},
                    {"type": {"name": "Relates"}, "outwardIssue": {"key": "PROJ-5"}}
                ]
            }},
            {"key": "PROJ-8", "fields": {"summary": "Sub", "status": {"name": "Backlog", "statusCategory": {"key": "new"}},
                                         "parent": {"key": "PROJ-7", "fields": {"issuetype": {"name": "Bug"}}}}},
            {"key": "PROJ-10"}
        ]});
        let export = read(&json.to_string(), Some("customfield_10016"))?;
        assert_eq!(export.issues.len(), 2);
        assert_eq!(export.warnings.len(), 1);
        let first = export.issues.first().cloned().unwrap_or_default();
        assert_eq!(first.category, Some(StateCategory::Canceled));
        assert!(first.canceled_at.is_some() && first.completed_at.is_none());
        assert_eq!(first.priority, Some(Priority::Urgent));
        assert_eq!(first.estimate.as_deref(), Some("5"));
        assert_eq!(first.project.as_deref(), Some("Auth revamp"));
        assert_eq!(first.parent, None);
        assert!(first.labels.contains("bug"));
        assert_eq!(first.assignee.as_deref(), Some("bo@northstar.dev"));
        assert_eq!(first.creator.as_deref(), Some("Ada"));
        assert_eq!(
            first.relations,
            vec![
                (Relation::Blocks, "PROJ-9".to_owned()),
                (Relation::BlockedBy, "PROJ-3".to_owned()),
                (Relation::Related, "PROJ-5".to_owned()),
            ]
        );
        let second = export.issues.get(1).cloned().unwrap_or_default();
        assert_eq!(second.category, Some(StateCategory::Backlog));
        assert_eq!(second.parent.as_deref(), Some("PROJ-7"));
        assert!(read("{}", None).is_err());
        Ok(())
    }
}
