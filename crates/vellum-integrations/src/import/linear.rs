// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Linear's CSV export (Settings → Import / export → Export CSV).
//!
//! Linear's export carries state names but not categories, so the
//! category comes from well-known names first, then from the lifecycle
//! timestamps (`Completed`, `Canceled`, `Started`).

use super::csv::Table;
use super::{Export, ImportError, ImportedIssue, Relation, Warning, timestamp};
use crate::model::{Priority, StateCategory};

fn category(name: &str) -> Option<StateCategory> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "triage" => StateCategory::Triage,
        "backlog" | "icebox" => StateCategory::Backlog,
        "todo" | "to do" | "unstarted" | "planned" => StateCategory::Unstarted,
        "in progress" | "in review" | "started" | "doing" => StateCategory::Started,
        "done" | "completed" | "closed" | "shipped" => StateCategory::Completed,
        "canceled" | "cancelled" | "won't do" | "wont do" => StateCategory::Canceled,
        "duplicate" => StateCategory::Duplicate,
        _ => return None,
    })
}

fn list(value: &str) -> impl Iterator<Item = String> + '_ {
    value
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

/// Reads a Linear CSV export.
///
/// # Errors
///
/// [`ImportError`] when the CSV is malformed or lacks `ID` or `Title`.
pub fn read(text: &str) -> Result<Export, ImportError> {
    let table = Table::parse(text)?;
    for column in ["ID", "Title"] {
        if !table.has(column) {
            return Err(ImportError::MissingColumn(column.to_owned()));
        }
    }
    let mut export = Export::default();
    for row in &table.rows {
        let id = table.get(row, &["ID"]).to_owned();
        if id.is_empty() {
            export.warnings.push(Warning {
                external_id: None,
                message: "row without an ID skipped".to_owned(),
            });
            continue;
        }
        let state_name = table.get(row, &["Status", "State"]).to_owned();
        let completed_at = timestamp(table.get(row, &["Completed"]));
        let canceled_at = timestamp(table.get(row, &["Canceled"]));
        let started_at = timestamp(table.get(row, &["Started"]));
        let inferred = category(&state_name).or_else(|| {
            if completed_at.is_some() {
                Some(StateCategory::Completed)
            } else if canceled_at.is_some() {
                Some(StateCategory::Canceled)
            } else if started_at.is_some() {
                Some(StateCategory::Started)
            } else {
                None
            }
        });
        let mut relations = Vec::new();
        for (column, relation) in [
            ("Blocks", Relation::Blocks),
            ("Blocked by", Relation::BlockedBy),
            ("Related to", Relation::Related),
            ("Duplicate of", Relation::DuplicateOf),
        ] {
            relations.extend(list(table.get(row, &[column])).map(|other| (relation, other)));
        }
        let due = table.get(row, &["Due Date"]);
        export.issues.push(ImportedIssue {
            external_id: id,
            title: table.get(row, &["Title"]).to_owned(),
            description: table.get(row, &["Description"]).to_owned(),
            state_name,
            category: inferred,
            priority: Priority::parse(table.get(row, &["Priority"])),
            estimate: table.opt(row, &["Estimate"]),
            labels: list(table.get(row, &["Labels"])).collect(),
            assignee: table.opt(row, &["Assignee"]),
            creator: table.opt(row, &["Creator"]),
            project: table.opt(row, &["Project"]),
            created_at: timestamp(table.get(row, &["Created"])),
            completed_at,
            canceled_at,
            started_at,
            due: due.get(..10).and_then(|d| d.parse().ok()),
            parent: table.opt(row, &["Parent issue", "Parent"]),
            relations,
        });
    }
    Ok(export)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "ID,Team,Title,Description,Status,Estimate,Priority,Project,Creator,Assignee,Labels,Created,Started,Completed,Canceled,Due Date,Parent issue,Blocked by\n\
NS-12,Northstar,Ship export,\"Use **Typst**, not Chromium\",In Review,3,High,Exports,ada@northstar.dev,bo@northstar.dev,\"export, pdf\",2026-01-05T10:00:00.000Z,2026-01-06T10:00:00.000Z,,,2026-02-01,NS-10,NS-11\n\
NS-13,Northstar,Old idea,,Someday,,No priority,,ada@northstar.dev,,,2026-01-07T10:00:00.000Z,,,2026-01-08T10:00:00.000Z,,,\n";

    #[test]
    fn reads_linear_csv() -> Result<(), ImportError> {
        let export = read(SAMPLE)?;
        assert_eq!(export.issues.len(), 2);
        let first = export.issues.first().cloned().unwrap_or_default();
        assert_eq!(first.external_id, "NS-12");
        assert_eq!(first.description, "Use **Typst**, not Chromium");
        assert_eq!(first.category, Some(StateCategory::Started));
        assert_eq!(first.priority, Some(Priority::High));
        assert_eq!(first.labels.len(), 2);
        assert_eq!(
            first.due.map(|d| d.to_string()).as_deref(),
            Some("2026-02-01")
        );
        assert_eq!(first.parent.as_deref(), Some("NS-10"));
        assert_eq!(
            first.relations,
            vec![(Relation::BlockedBy, "NS-11".to_owned())]
        );
        let second = export.issues.get(1).cloned().unwrap_or_default();
        // An unknown state name falls back to the lifecycle timestamps.
        assert_eq!(second.category, Some(StateCategory::Canceled));
        assert_eq!(second.priority, Some(Priority::None));
        Ok(())
    }

    #[test]
    fn requires_id_and_title() {
        assert_eq!(
            read("Title\nx\n"),
            Err(ImportError::MissingColumn("ID".to_owned()))
        );
    }
}
