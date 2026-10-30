// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! One-way import from Google Tasks, Todoist and Things (R-MW-6, D84).
//!
//! Each source list becomes a checklist file under `tasks/<user>/`. Every
//! imported item carries `src=<source>:<id>`, so importing the same export
//! again only adds what is new. The formats read are the ones each app
//! hands its users:
//!
//! | Source | Input | Notes |
//! |---|---|---|
//! | Google Tasks | `Tasks.json` from Google Takeout (or one list from the Tasks API) | due dates only: Google discards the time |
//! | Todoist | a project's CSV template export | dates are Todoist's English phrases, read by quick-add |
//! | Things 3 | Things JSON (the documented `things:///json` schema) | `when` becomes the start date, `deadline` the deadline |

use std::collections::HashSet;
use std::fmt::Write as _;

use jiff::civil::{Date, DateTime, Time};
use jiff::{Timestamp, ToSpan as _};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use vellum_kernel::{BlockId, DocId};

use crate::checklist::{self, Checklist};
use crate::model::{Due, Priority, Reminder, TaskFields, When};
use crate::quickadd;
use crate::telemetry;

/// Where an import came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    /// Google Tasks.
    GoogleTasks,
    /// Todoist.
    Todoist,
    /// Things 3.
    Things,
}

impl Source {
    /// The `src` prefix and metric label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::GoogleTasks => "gtasks",
            Self::Todoist => "todoist",
            Self::Things => "things",
        }
    }
}

/// Why an export could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The JSON does not have the expected shape.
    #[error("cannot read the export: {0}")]
    Json(#[from] serde_json::Error),
    /// The CSV does not have the expected shape.
    #[error("cannot read the export: {0}")]
    Csv(#[from] csv::Error),
    /// A required column is missing.
    #[error("the CSV has no {0} column")]
    Column(&'static str),
}

/// One imported task with its subtasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The title.
    pub title: String,
    /// Notes, kept as indented lines under the item.
    pub notes: Option<String>,
    /// Whether it was completed.
    pub done: bool,
    /// The fields, with `src` set.
    pub fields: TaskFields,
    /// Subtasks.
    pub children: Vec<Self>,
}

/// A run of items under an optional heading.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Section {
    /// The heading (a Todoist section or a Things heading).
    pub heading: Option<String>,
    /// The items.
    pub items: Vec<Item>,
}

/// One imported list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List {
    /// The list title.
    pub title: String,
    /// The list's sections, the first usually without a heading.
    pub sections: Vec<Section>,
}

impl List {
    /// How many items the list holds, subtasks included.
    #[must_use]
    pub fn count(&self) -> usize {
        self.sections.iter().map(|s| count_items(&s.items)).sum()
    }
}

/// Escapes text so it stays plain text inside a checklist item: no raw
/// HTML, and nothing that reads as an attribute span or block ID.
fn escape_inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
    {
        if matches!(c, '<' | '{' | '^' | '[' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn short_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0x1f]);
    }
    hasher
        .finalize()
        .iter()
        .take(8)
        .fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

fn write_items(out: &mut String, items: &[Item], indent: usize) {
    for item in items {
        let line = checklist::render_item(
            indent,
            item.done,
            &escape_inline(&item.title),
            &item.fields,
            BlockId::generate(),
        );
        out.push_str(&line);
        out.push('\n');
        if let Some(notes) =
            item.notes.as_deref().filter(|n| !n.trim().is_empty())
        {
            for note in notes.lines() {
                let note = note.trim_end();
                if note.trim().is_empty() {
                    continue;
                }
                let _ = writeln!(
                    out,
                    "{}{}",
                    " ".repeat(indent + 2),
                    escape_note(note.trim_start())
                );
            }
        }
        write_items(out, &item.children, indent + 2);
    }
}

fn escape_note(text: &str) -> String {
    let mut out = String::new();
    // A note line must not start a list item or a heading of its own.
    if text.starts_with(['-', '*', '+', '#', '>'])
        || text.starts_with(|c: char| c.is_ascii_digit())
    {
        out.push('\\');
    }
    for c in text.chars() {
        if matches!(c, '<' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn yaml_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Writes a new list file: front matter and the checklist.
#[must_use]
pub fn render_list(list: &List, id: DocId, owner: &str) -> String {
    let mut out = format!(
        "---\nid: {id}\ntitle: {}\nowner: {}\nvellum: 1\n---\n",
        yaml_string(&list.title),
        yaml_string(&format!("@{owner}"))
    );
    for section in &list.sections {
        if section.items.is_empty() {
            continue;
        }
        out.push('\n');
        if let Some(heading) = &section.heading {
            let _ = writeln!(out, "## {}\n", escape_inline(heading));
        }
        write_items(&mut out, &section.items, 0);
    }
    out
}

/// Re-import: adds the items whose `src` is not in the file yet.
///
/// New subtasks of an item imported before go under it; other new items
/// go at the end. Returns the new file and how many items were added, or
/// `None` when nothing is new.
#[must_use]
pub fn append_new(existing: &str, list: &List) -> Option<(String, usize)> {
    let mut out = existing.to_owned();
    let mut added = 0;
    let mut tail = List {
        title: list.title.clone(),
        sections: Vec::new(),
    };
    for section in &list.sections {
        let mut fresh = Vec::new();
        for item in &section.items {
            added += merge_item(&mut out, item, None, &mut fresh);
        }
        if !fresh.is_empty() {
            tail.sections.push(Section {
                heading: section.heading.clone(),
                items: fresh,
            });
        }
    }
    if added == 0 {
        return None;
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    for section in &tail.sections {
        out.push('\n');
        if let Some(heading) = &section.heading {
            let _ = writeln!(out, "## {}\n", escape_inline(heading));
        }
        write_items(&mut out, &section.items, 0);
    }
    Some((out, added))
}

/// Merges one item: returns how many items it added. A new top-level
/// item is collected into `fresh`; a new subtask of a known item is
/// inserted under it.
fn merge_item(
    out: &mut String,
    item: &Item,
    parent: Option<BlockId>,
    fresh: &mut Vec<Item>,
) -> usize {
    let existing = Checklist::parse(out)
        .items
        .into_iter()
        .find(|i| i.fields.src.is_some() && i.fields.src == item.fields.src)
        .and_then(|i| i.block);
    match (existing, parent) {
        (Some(block), _) => item
            .children
            .iter()
            .map(|child| merge_item(out, child, Some(block), fresh))
            .sum(),
        (None, Some(parent)) => {
            let block = BlockId::generate();
            let title = escape_inline(&item.title);
            let mut fields = item.fields.clone();
            if !item.done {
                fields.done = None;
            }
            if let Ok(next) =
                checklist::append(out, Some(parent), &title, &fields, block)
            {
                *out = next;
            }
            1 + item
                .children
                .iter()
                .map(|child| merge_item(out, child, Some(block), fresh))
                .sum::<usize>()
        }
        (None, None) => {
            fresh.push(item.clone());
            1 + count_items(&item.children)
        }
    }
}

fn count_items(items: &[Item]) -> usize {
    items.iter().map(|i| 1 + count_items(&i.children)).sum()
}

fn finish(source: Source, lists: Vec<List>) -> Vec<List> {
    let total: usize = lists.iter().map(List::count).sum();
    telemetry::record_import(source.as_str(), "read", total);
    tracing::info!(
        source = source.as_str(),
        lists = lists.len(),
        items = total,
        "tasks.import"
    );
    lists
}

// ---------------------------------------------------------------- Google

#[derive(Debug, Deserialize)]
struct GoogleExport {
    #[serde(default)]
    kind: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    items: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct GoogleTask {
    id: String,
    title: String,
    notes: Option<String>,
    status: String,
    due: Option<String>,
    completed: Option<String>,
    parent: Option<String>,
    position: String,
    deleted: bool,
}

fn google_list(
    title: &str,
    items: &[serde_json::Value],
) -> Result<List, ImportError> {
    let mut tasks: Vec<GoogleTask> = items
        .iter()
        .map(|v| serde_json::from_value(v.clone()))
        .collect::<Result<_, _>>()?;
    tasks.retain(|t| !t.deleted && !t.title.trim().is_empty());
    tasks.sort_by(|a, b| a.position.cmp(&b.position));
    let ids: HashSet<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
    let to_item = |t: &GoogleTask| {
        let done = t.status == "completed";
        Item {
            title: t.title.clone(),
            notes: t.notes.clone(),
            done,
            fields: TaskFields {
                // Google keeps only the day of a due date.
                due: t
                    .due
                    .as_deref()
                    .and_then(|d| d.get(..10))
                    .and_then(|d| d.parse::<Date>().ok())
                    .map(Due::Date),
                done: t
                    .completed
                    .as_deref()
                    .and_then(|c| c.parse::<Timestamp>().ok())
                    .filter(|_| done),
                src: Some(format!("gtasks:{}", t.id)),
                ..TaskFields::default()
            },
            children: Vec::new(),
        }
    };
    let mut roots: Vec<Item> = Vec::new();
    for task in tasks
        .iter()
        .filter(|t| t.parent.as_deref().is_none_or(|p| !ids.contains(p)))
    {
        let mut item = to_item(task);
        item.children = tasks
            .iter()
            .filter(|c| c.parent.as_deref() == Some(task.id.as_str()))
            .map(to_item)
            .collect();
        roots.push(item);
    }
    Ok(List {
        title: title.to_owned(),
        sections: vec![Section {
            heading: None,
            items: roots,
        }],
    })
}

/// Reads Google Takeout's `Tasks.json`, or one list from the Tasks API.
///
/// # Errors
///
/// Returns [`ImportError`] when the JSON has another shape.
pub fn google_tasks(json: &str) -> Result<Vec<List>, ImportError> {
    let _span = tracing::info_span!("tasks.import.google").entered();
    let export: GoogleExport = serde_json::from_str(json)?;
    let lists = if export.kind == "tasks#tasks" {
        vec![google_list(
            export.title.as_deref().unwrap_or("Google Tasks"),
            &export.items,
        )?]
    } else {
        export
            .items
            .iter()
            .map(|list| {
                let title = list
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("Google Tasks");
                let items = list
                    .get("items")
                    .and_then(serde_json::Value::as_array)
                    .map_or(&[][..], Vec::as_slice);
                google_list(title, items)
            })
            .collect::<Result<_, _>>()?
    };
    Ok(finish(Source::GoogleTasks, lists))
}

// --------------------------------------------------------------- Todoist

/// The columns of Todoist's CSV template.
struct TodoistColumns {
    kind: usize,
    content: usize,
    description: Option<usize>,
    priority: Option<usize>,
    indent: Option<usize>,
    date: Option<usize>,
    lang: Option<usize>,
    deadline: Option<usize>,
}

impl TodoistColumns {
    fn find(headers: &csv::StringRecord) -> Result<Self, ImportError> {
        let column = |name: &'static str| {
            headers
                .iter()
                .position(|h| h.trim().eq_ignore_ascii_case(name))
                .ok_or(ImportError::Column(name))
        };
        Ok(Self {
            kind: column("TYPE")?,
            content: column("CONTENT")?,
            description: column("DESCRIPTION").ok(),
            priority: column("PRIORITY").ok(),
            indent: column("INDENT").ok(),
            date: column("DATE").ok(),
            lang: column("DATE_LANG").ok(),
            deadline: column("DEADLINE").ok(),
        })
    }
}

fn cell(record: &csv::StringRecord, column: Option<usize>) -> &str {
    column.and_then(|i| record.get(i)).unwrap_or("").trim()
}

/// Builds one Todoist task row; returns it with its indent level (1-5).
fn todoist_item(
    columns: &TodoistColumns,
    record: &csv::StringRecord,
    project: &str,
    row: usize,
    now: DateTime,
) -> (Item, usize) {
    let content = cell(record, Some(columns.content));
    let mut labels = Vec::new();
    let mut words = Vec::new();
    for word in content.split_whitespace() {
        match word.strip_prefix('@').filter(|l| !l.is_empty()) {
            Some(label) => labels.push(label.to_lowercase()),
            None => words.push(word),
        }
    }
    let mut notes = Some(cell(record, columns.description).to_owned())
        .filter(|d| !d.is_empty());
    let mut fields = TaskFields {
        labels,
        src: Some(format!(
            "todoist:{}",
            short_hash(&[project, content, &row.to_string()])
        )),
        // The template's PRIORITY is the p-level shown in the app.
        priority: cell(record, columns.priority)
            .parse::<u8>()
            .ok()
            .and_then(Priority::new),
        ..TaskFields::default()
    };
    let date = cell(record, columns.date);
    let lang = cell(record, columns.lang);
    if !date.is_empty() {
        let parsed = quickadd::parse(date, now);
        if (lang.is_empty() || lang.eq_ignore_ascii_case("en"))
            && parsed.title.is_empty()
        {
            fields.due = parsed.fields.due;
            fields.recurrence = parsed.fields.recurrence;
        } else {
            let line = format!("Todoist date: {date}");
            notes = Some(
                notes.map_or_else(|| line.clone(), |n| format!("{n}\n{line}")),
            );
        }
    }
    if let Ok(deadline) = cell(record, columns.deadline).parse::<Date>() {
        // Todoist's deadline is the hard date; its date is the plan.
        fields.start = fields.due.map(Due::date);
        fields.due = Some(Due::Date(deadline));
    }
    let level = cell(record, columns.indent)
        .parse::<usize>()
        .unwrap_or(1)
        .clamp(1, 5);
    let item = Item {
        title: words.join(" "),
        notes,
        done: false,
        fields,
        children: Vec::new(),
    };
    (item, level)
}

/// Reads a Todoist project's CSV template export.
///
/// Dates are Todoist's own phrases ("every monday", "Nov 3 2026 15:00"),
/// read with quick-add as of `now`; a phrase quick-add cannot read is
/// kept in the notes.
///
/// # Errors
///
/// Returns [`ImportError`] for malformed CSV or missing columns.
pub fn todoist_csv(
    project: &str,
    csv_text: &str,
    now: DateTime,
) -> Result<Vec<List>, ImportError> {
    let _span = tracing::info_span!("tasks.import.todoist").entered();
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(csv_text.as_bytes());
    let columns = TodoistColumns::find(reader.headers()?)?;
    let mut sections = vec![Section::default()];
    // Path of open items by indent level, as indices into the section.
    let mut stack: Vec<Vec<usize>> = Vec::new();
    for (row, record) in reader.records().enumerate() {
        let record = record?;
        let Some(section) = sections.last_mut() else {
            continue;
        };
        match cell(&record, Some(columns.kind)) {
            "section" => {
                sections.push(Section {
                    heading: Some(
                        cell(&record, Some(columns.content)).to_owned(),
                    ),
                    items: Vec::new(),
                });
                stack.clear();
            }
            "note" => {
                let target = stack
                    .last()
                    .and_then(|path| item_at(&mut section.items, path));
                if let Some(item) = target {
                    let notes = item.notes.get_or_insert_with(String::new);
                    if !notes.is_empty() {
                        notes.push('\n');
                    }
                    notes.push_str(cell(&record, Some(columns.content)));
                }
            }
            "task" => {
                let (item, level) =
                    todoist_item(&columns, &record, project, row, now);
                stack.truncate(level - 1);
                let parent = stack.last().cloned();
                let path = if let Some((mut path, parent)) =
                    parent.and_then(|p| {
                        item_at(&mut section.items, &p).map(|i| (p, i))
                    }) {
                    parent.children.push(item);
                    path.push(parent.children.len() - 1);
                    path
                } else {
                    section.items.push(item);
                    vec![section.items.len() - 1]
                };
                stack.push(path);
            }
            _ => {}
        }
    }
    let list = List {
        title: project.to_owned(),
        sections,
    };
    Ok(finish(Source::Todoist, vec![list]))
}

fn item_at<'a>(items: &'a mut [Item], path: &[usize]) -> Option<&'a mut Item> {
    let (first, rest) = path.split_first()?;
    let item = items.get_mut(*first)?;
    if rest.is_empty() {
        Some(item)
    } else {
        item_at(&mut item.children, rest)
    }
}

// ---------------------------------------------------------------- Things

#[derive(Debug, Deserialize)]
struct ThingsObject {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    attributes: ThingsAttributes,
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, rename_all = "kebab-case")]
struct ThingsAttributes {
    title: String,
    notes: Option<String>,
    when: Option<String>,
    deadline: Option<String>,
    tags: Vec<String>,
    completed: bool,
    canceled: bool,
    checklist_items: Vec<ThingsObject>,
    items: Vec<ThingsObject>,
}

fn things_when(when: &str, today: Date, fields: &mut TaskFields) {
    let (day, time) = match when.split_once('@') {
        Some((day, time)) => (day, Some(time)),
        None => (when, None),
    };
    let date = match day.trim().to_ascii_lowercase().as_str() {
        "today" => Some(today),
        "tonight" | "evening" => {
            fields.when = Some(When::Evening);
            Some(today)
        }
        "tomorrow" => today.checked_add(1.day()).ok(),
        "anytime" => {
            fields.when = Some(When::Anytime);
            None
        }
        "someday" => {
            fields.when = Some(When::Someday);
            None
        }
        other => other.parse::<Date>().ok(),
    };
    fields.start = date;
    // Things' "when@time" is a reminder at that time.
    if let (Some(date), Some(time)) =
        (date, time.and_then(|t| t.trim().parse::<Time>().ok()))
    {
        fields.reminders.push(Reminder::At(date.to_datetime(time)));
    }
}

fn things_todo(
    object: &ThingsObject,
    today: Date,
    scope: &str,
    index: usize,
) -> Item {
    let a = &object.attributes;
    let id = object
        .id
        .clone()
        .unwrap_or_else(|| short_hash(&[scope, &a.title, &index.to_string()]));
    let mut fields = TaskFields {
        labels: a.tags.iter().map(|t| t.to_lowercase()).collect(),
        due: a
            .deadline
            .as_deref()
            .and_then(|d| d.parse::<Date>().ok())
            .map(Due::Date),
        src: Some(format!("things:{id}")),
        ..TaskFields::default()
    };
    if let Some(when) = &a.when {
        things_when(when, today, &mut fields);
    }
    let children = a
        .checklist_items
        .iter()
        .enumerate()
        .map(|(i, c)| Item {
            title: c.attributes.title.clone(),
            notes: None,
            done: c.attributes.completed,
            fields: TaskFields {
                src: Some(format!("things:{id}:{i}")),
                ..TaskFields::default()
            },
            children: Vec::new(),
        })
        .collect();
    Item {
        title: a.title.clone(),
        notes: a.notes.clone(),
        done: a.completed || a.canceled,
        fields,
        children,
    }
}

/// Reads Things JSON (the documented `things:///json` schema).
///
/// To-dos at the top level go to a "Things" list; each project becomes a list,
/// its headings become headings, and checklist items become subtasks.
///
/// # Errors
///
/// Returns [`ImportError`] when the JSON has another shape.
pub fn things_json(json: &str, today: Date) -> Result<Vec<List>, ImportError> {
    let _span = tracing::info_span!("tasks.import.things").entered();
    let objects: Vec<ThingsObject> = serde_json::from_str(json)?;
    let mut loose = List {
        title: "Things".into(),
        sections: vec![Section::default()],
    };
    let mut lists = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        match object.kind.as_str() {
            "to-do" => {
                if let Some(section) = loose.sections.first_mut() {
                    section.items.push(things_todo(object, today, "", index));
                }
            }
            "project" => {
                let title = &object.attributes.title;
                let mut sections = vec![Section::default()];
                for (i, child) in object.attributes.items.iter().enumerate() {
                    match child.kind.as_str() {
                        "heading" => sections.push(Section {
                            heading: Some(child.attributes.title.clone()),
                            items: Vec::new(),
                        }),
                        "to-do" => {
                            if let Some(section) = sections.last_mut() {
                                section
                                    .items
                                    .push(things_todo(child, today, title, i));
                            }
                        }
                        _ => {}
                    }
                }
                lists.push(List {
                    title: title.clone(),
                    sections,
                });
            }
            _ => {}
        }
    }
    if loose.count() > 0 {
        lists.insert(0, loose);
    }
    Ok(finish(Source::Things, lists))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Origin;
    use crate::model::RecurFrom;
    use crate::projection::{FileContext, project_file};
    use jiff::civil::date;

    fn doc() -> DocId {
        "01M639NFH15EBMG6C7NX1ZK4KA".parse().unwrap()
    }

    #[test]
    fn google_takeout() {
        let json = r#"{"kind":"tasks#taskLists","items":[{"kind":"tasks#tasks","title":"Errands","items":[
          {"id":"b","title":"Child","status":"needsAction","parent":"a","position":"001"},
          {"id":"a","title":"Buy <milk> {soon}","notes":"- semi-skimmed\n2 litres","status":"needsAction","due":"2026-11-03T00:00:00.000Z","position":"000"},
          {"id":"c","title":"Done one","status":"completed","completed":"2026-10-01T10:00:00.000Z","position":"002"},
          {"id":"d","title":"Gone","status":"needsAction","deleted":true,"position":"003"}]}]}"#;
        let lists = google_tasks(json).unwrap();
        assert_eq!(lists.len(), 1);
        let text = render_list(&lists[0], doc(), "gourav");
        assert!(text.starts_with(
            "---\nid: 01M639NFH15EBMG6C7NX1ZK4KA\n\
            title: \"Errands\"\nowner: \"@gourav\"\nvellum: 1\n---\n\n"
        ));
        assert!(text.contains(
            "- [ ] Buy \\<milk> \\{soon} {due=2026-11-03 src=gtasks:a} ^blk-"
        ));
        assert!(text.contains(
            "\n  \\- semi-skimmed\n  \\2 litres\n  - [ ] Child \
                {src=gtasks:b} ^blk-"
        ));
        assert!(text.contains(
            "- [x] Done one {done=2026-10-01T10:00:00Z src=gtasks:c} ^blk-"
        ));
        assert!(!text.contains("Gone"));
        // The file reads back as tasks with the right shape.
        let context = FileContext {
            doc: doc(),
            origin: Origin::List,
            owner: None,
            list: None,
        };
        let tasks = project_file(&context, &text);
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].title, "Buy \\<milk> \\{soon}");
        assert!(tasks[1].parent.is_some());
        assert!(tasks[2].done);
        // Importing again adds nothing; a new task is appended.
        assert_eq!(append_new(&text, &lists[0]), None);
        let more = json.replace(
            concat!(
                r#"{"id":"d","title":"Gone","status":"needsAction","deleted":"#,
                r#"true,"position":"003"}"#,
            ),
            concat!(
                r#"{"id":"e","title":"New","status":"needsAction","position":"#,
                r#""004"}"#,
            ),
        );
        let (updated, added) =
            append_new(&text, &google_tasks(&more).unwrap()[0]).unwrap();
        assert_eq!(added, 1);
        assert!(updated.starts_with(&text));
        assert!(updated.contains("\n\n- [ ] New {src=gtasks:e} ^blk-"));
        // A new subtask of a known task goes under it.
        let child = more.replace(r#"{"id":"e","#, concat!(
            r#"{"id":"f","title":"Second "#,
            r#"child","status":"needsAction","parent":"a","position":"0015"},"#,
            r#"{"id":"e","#,
        ));
        let (nested, added) =
            append_new(&updated, &google_tasks(&child).unwrap()[0]).unwrap();
        assert_eq!(added, 1);
        assert!(nested.contains("  - [ ] Child {src=gtasks:b} ^blk-"));
        assert!(nested.contains("  - [ ] Second child {src=gtasks:f} ^blk-"));
        let tasks = project_file(&context, &nested);
        let second = tasks.iter().find(|t| t.title == "Second child").unwrap();
        let buy = tasks.iter().find(|t| t.title.starts_with("Buy")).unwrap();
        let crate::model::Source::Checklist { block, .. } = buy.source else {
            panic!()
        };
        assert_eq!(second.parent, Some(block));
    }

    #[test]
    fn google_api_list_and_bad_json() {
        let json = concat!(
            r#"{"kind":"tasks#tasks","items":[{"id":"x","title":"One","status"#,
            r#"":"needsAction","position":"0"}]}"#,
        );
        let lists = google_tasks(json).unwrap();
        assert_eq!(lists[0].title, "Google Tasks");
        assert!(google_tasks("[1,2]").is_err());
    }

    #[test]
    fn todoist_template() {
        let csv = "TYPE,CONTENT,DESCRIPTION,PRIORITY,INDENT,AUTHOR,RESPONSIBLE,\
DATE,DATE_LANG,TIMEZONE\ntask,Water plants @home,,4,1,,,every! 3 days,en,\n\
task,Pick pots,,1,2,,,,,\nnote,Terracotta,,,,,,,,\nsection,Later,,,,,,,,\n\
task,Plan trip,Ideas,2,1,,,Nov 20 2026,en,\ntask,Fête,,,1,,,demain,fr,\n";
        let now = date(2026, 11, 2).at(9, 0, 0, 0);
        let lists = todoist_csv("Garden", csv, now).unwrap();
        let list = &lists[0];
        assert_eq!(list.sections.len(), 2);
        let water = &list.sections[0].items[0];
        assert_eq!(water.title, "Water plants");
        assert_eq!(water.fields.labels, ["home"]);
        assert_eq!(water.fields.priority, Priority::new(4));
        let rec = water.fields.recurrence.as_ref().unwrap();
        assert_eq!(rec.from, RecurFrom::Completion);
        assert_eq!(water.children[0].title, "Pick pots");
        assert_eq!(water.children[0].notes.as_deref(), Some("Terracotta"));
        let later = &list.sections[1];
        assert_eq!(later.heading.as_deref(), Some("Later"));
        assert_eq!(
            later.items[0].fields.due.unwrap().to_string(),
            "2026-11-20"
        );
        assert_eq!(
            later.items[1].notes.as_deref(),
            Some("Todoist date: demain")
        );
        let text = render_list(list, doc(), "gourav");
        assert!(text.contains(
            "\n## Later\n\n- [ ] Plan trip {due=2026-11-20 p=2 src=todoist:"
        ));
        assert!(todoist_csv("x", "A,B\n1,2\n", now).is_err());
    }

    #[test]
    fn things_projects_headings_and_checklists() {
        let json = r#"[
          {"type":"to-do","attributes":{"title":"Loose","when":"evening","tags":["Home"]}},
          {"type":"project","attributes":{"title":"Launch","items":[
            {"type":"to-do","id":"T1","attributes":{"title":"Draft","when":"2026-11-04@09:30","deadline":"2026-11-06",
              "checklist-items":[{"type":"checklist-item","attributes":{"title":"Outline","completed":true}}]}},
            {"type":"heading","attributes":{"title":"Ship"}},
            {"type":"to-do","attributes":{"title":"Post","when":"someday","completed":true}}]}}]"#;
        let today = date(2026, 11, 2);
        let lists = things_json(json, today).unwrap();
        assert_eq!(
            lists.iter().map(|l| l.title.as_str()).collect::<Vec<_>>(),
            ["Things", "Launch"]
        );
        let loose = &lists[0].sections[0].items[0];
        assert_eq!(loose.fields.when, Some(When::Evening));
        assert_eq!(loose.fields.start, Some(today));
        assert_eq!(loose.fields.labels, ["home"]);
        let draft = &lists[1].sections[0].items[0];
        assert_eq!(draft.fields.start, Some(date(2026, 11, 4)));
        assert_eq!(draft.fields.due, Some(Due::Date(date(2026, 11, 6))));
        assert_eq!(draft.fields.reminders[0].to_string(), "2026-11-04T09:30");
        assert_eq!(draft.fields.src.as_deref(), Some("things:T1"));
        assert!(draft.children[0].done);
        let ship = &lists[1].sections[1];
        assert_eq!(ship.heading.as_deref(), Some("Ship"));
        assert!(ship.items[0].done);
        assert!(things_json("{}", today).is_err());
    }
}
