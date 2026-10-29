// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! GitHub Issues, as the REST API lists them
//! (`GET /repos/{owner}/{repo}/issues?state=all`), saved as a JSON array.
//!
//! Pull requests in the list are skipped. Closed issues map by
//! `state_reason`: `completed` → completed, `not_planned` → canceled,
//! `duplicate` → duplicate. The milestone becomes the project. Sub-issues
//! come from `parent_issue_url` when present.

use serde_json::Value;

use super::{Export, ImportError, ImportedIssue, timestamp};
use crate::inbound::text;
use crate::model::StateCategory;

/// Reads a GitHub issues listing for `repo` (`owner/name`).
///
/// # Errors
///
/// [`ImportError::Unreadable`] if the JSON is not an array.
pub fn read(json: &str, repo: &str) -> Result<Export, ImportError> {
    let value: Value =
        serde_json::from_str(json).map_err(|e| ImportError::Unreadable(e.to_string()))?;
    let list = value
        .as_array()
        .ok_or_else(|| ImportError::Unreadable("expected a JSON array of issues".to_owned()))?;
    let mut export = Export::default();
    for issue in list {
        if issue.get("pull_request").is_some() {
            continue;
        }
        let Some(number) = issue.get("number").and_then(Value::as_u64) else {
            continue;
        };
        let closed = text(issue, "/state") == "closed";
        let category = if closed {
            match text(issue, "/state_reason") {
                "not_planned" => StateCategory::Canceled,
                "duplicate" => StateCategory::Duplicate,
                _ => StateCategory::Completed,
            }
        } else if issue
            .get("assignees")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
        {
            StateCategory::Unstarted
        } else {
            StateCategory::Backlog
        };
        let closed_at = timestamp(text(issue, "/closed_at"));
        let parent = issue
            .get("parent_issue_url")
            .and_then(Value::as_str)
            .and_then(|url| url.rsplit('/').next())
            .and_then(|n| n.parse::<u64>().ok())
            .map(|n| format!("{repo}#{n}"));
        export.issues.push(ImportedIssue {
            external_id: format!("{repo}#{number}"),
            title: text(issue, "/title").to_owned(),
            description: text(issue, "/body").to_owned(),
            state_name: text(issue, "/state").to_owned(),
            category: Some(category),
            priority: None,
            estimate: None,
            labels: issue
                .get("labels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|l| l.as_str().unwrap_or_else(|| text(l, "/name")).to_owned())
                .filter(|l| !l.is_empty())
                .collect(),
            assignee: issue
                .get("assignees")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .map(|a| text(a, "/login").to_owned())
                .filter(|l| !l.is_empty()),
            creator: Some(text(issue, "/user/login").to_owned()).filter(|l| !l.is_empty()),
            project: Some(text(issue, "/milestone/title").to_owned()).filter(|m| !m.is_empty()),
            created_at: timestamp(text(issue, "/created_at")),
            completed_at: closed_at.filter(|_| category == StateCategory::Completed),
            canceled_at: closed_at.filter(|_| category != StateCategory::Completed),
            started_at: None,
            due: None,
            parent,
            relations: Vec::new(),
        });
    }
    Ok(export)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_issues_and_skips_pull_requests() -> Result<(), ImportError> {
        let json = json!([
            {"number": 3, "title": "Crash on start", "body": "Steps…", "state": "closed", "state_reason": "not_planned",
             "labels": [{"name": "bug"}, "p1"], "assignees": [{"login": "adap"}], "user": {"login": "bo"},
             "milestone": {"title": "1.0"}, "created_at": "2026-01-01T00:00:00Z", "closed_at": "2026-01-02T00:00:00Z",
             "parent_issue_url": "https://api.github.com/repos/northstar/api/issues/1"},
            {"number": 4, "title": "A PR", "state": "open", "pull_request": {}},
            {"number": 5, "title": "Idea", "state": "open", "assignees": []}
        ]);
        let export = read(&json.to_string(), "northstar/api")?;
        assert_eq!(export.issues.len(), 2);
        let first = export.issues.first().cloned().unwrap_or_default();
        assert_eq!(first.external_id, "northstar/api#3");
        assert_eq!(first.category, Some(StateCategory::Canceled));
        assert!(first.canceled_at.is_some());
        assert_eq!(first.labels.len(), 2);
        assert_eq!(first.assignee.as_deref(), Some("adap"));
        assert_eq!(first.project.as_deref(), Some("1.0"));
        assert_eq!(first.parent.as_deref(), Some("northstar/api#1"));
        assert_eq!(
            export.issues.get(1).and_then(|i| i.category),
            Some(StateCategory::Backlog)
        );
        assert!(read("{}", "a/b").is_err());
        Ok(())
    }
}
