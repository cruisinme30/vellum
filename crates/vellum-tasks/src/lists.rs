// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Personal and shared lists, and assignment from comments (R-MW-7).
//!
//! A list is a checklist file under `tasks/<owner>/`, with front matter:
//!
//! ```text
//! ---
//! id: 01M639NFH15EBMG6C7NX1ZK4KA
//! title: "Groceries"
//! owner: "@gourav"
//! members: ["@ann", "@bob"]
//! vellum: 1
//! ---
//! ```
//!
//! Sharing grants the member edit access through the authorization
//! service (the source of truth for access, brief section 6) and records
//! them in `members` as the readable copy. Each item belongs to the
//! people it mentions, or to the owner when it mentions nobody, so a
//! shared list never floods every member's My Work.

use std::fmt::Write as _;

use vellum_kernel::{CommentId, DocId};

use crate::model::{Source, Task, TaskFields};

/// The folder that holds every user's lists.
pub const TASKS_DIR: &str = "tasks";

/// The file name of a user's Inbox list.
pub const INBOX_FILE: &str = "inbox.md";

/// A list's front matter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListMeta {
    /// The list's document ID.
    pub id: Option<DocId>,
    /// The title.
    pub title: String,
    /// The owner's handle, without `@`.
    pub owner: Option<String>,
    /// Members' handles, without `@`.
    pub members: Vec<String>,
}

/// A file-name slug for a list title: lower case, words joined by `-`.
#[must_use]
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for word in title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        if !out.is_empty() {
            out.push('-');
        }
        out.extend(word.chars().flat_map(char::to_lowercase));
    }
    if out.is_empty() {
        out.push_str("list");
    }
    out
}

/// The path of a list file, relative to the workspace root.
#[must_use]
pub fn list_path(user: &str, title: &str) -> String {
    format!("{TASKS_DIR}/{}/{}.md", slug(user), slug(title))
}

/// The path of a user's Inbox list.
#[must_use]
pub fn inbox_path(user: &str) -> String {
    format!("{TASKS_DIR}/{}/{INBOX_FILE}", slug(user))
}

fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

fn unquote(text: &str) -> String {
    let text = text.trim();
    let inner = text
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .or_else(|| text.strip_prefix('\'').and_then(|t| t.strip_suffix('\'')));
    inner.map_or_else(
        || text.to_owned(),
        |inner| inner.replace("\\\"", "\"").replace("\\\\", "\\"),
    )
}

fn handle(text: &str) -> String {
    unquote(text).trim_start_matches('@').to_ascii_lowercase()
}

fn members_line(members: &[String]) -> String {
    let quoted: Vec<String> =
        members.iter().map(|m| quote(&format!("@{m}"))).collect();
    format!("members: [{}]", quoted.join(", "))
}

/// Writes a new, empty list file.
#[must_use]
pub fn new_list(id: DocId, title: &str, owner: &str) -> String {
    format!(
        "---\nid: {id}\ntitle: {}\nowner: {}\nvellum: 1\n---\n\n",
        quote(title),
        quote(&format!("@{owner}"))
    )
}

/// The byte range of the front matter's body (between the fences).
fn front_matter(source: &str) -> Option<(usize, usize)> {
    let body_start = source
        .strip_prefix("---\n")
        .map(|_| 4)
        .or_else(|| source.strip_prefix("---\r\n").map(|_| 5))?;
    let mut at = body_start;
    for line in source.get(body_start..)?.split_inclusive('\n') {
        if line.trim_end() == "---" || line.trim_end() == "..." {
            return Some((body_start, at));
        }
        at += line.len();
    }
    None
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct FrontMatter {
    id: Option<String>,
    title: Option<String>,
    owner: Option<String>,
    members: Vec<String>,
}

/// Reads a list's front matter: `id`, `title`, `owner` and `members`.
/// Missing or unreadable front matter gives an empty [`ListMeta`].
#[must_use]
pub fn read_meta(source: &str) -> ListMeta {
    let front: FrontMatter = front_matter(source)
        .and_then(|(start, end)| source.get(start..end))
        .and_then(|yaml| serde_saphyr::from_str(yaml).ok())
        .unwrap_or_default();
    ListMeta {
        id: front.id.and_then(|id| id.parse().ok()),
        title: front.title.unwrap_or_default(),
        owner: front.owner.map(|o| handle(&o)).filter(|h| !h.is_empty()),
        members: front
            .members
            .iter()
            .map(|m| handle(m))
            .filter(|h| !h.is_empty())
            .collect(),
    }
}

/// Rewrites the `members` key, keeping every other line of the front
/// matter as it is.
fn write_members(source: &str, members: &[String]) -> Option<String> {
    let (start, end) = front_matter(source)?;
    let body = source.get(start..end)?;
    let mut out_lines: Vec<String> = Vec::new();
    let mut skipping_block = false;
    let mut written = false;
    for line in body.lines() {
        if skipping_block {
            if line.trim_start().starts_with("- ") {
                continue;
            }
            skipping_block = false;
        }
        if line
            .split_once(':')
            .is_some_and(|(k, _)| k.trim() == "members")
        {
            skipping_block = line.trim_end().ends_with(':');
            if !members.is_empty() {
                out_lines.push(members_line(members));
            }
            written = true;
            continue;
        }
        out_lines.push(line.to_owned());
    }
    if !written && !members.is_empty() {
        // After `owner`, or at the end.
        let at = out_lines
            .iter()
            .position(|l| l.starts_with("owner:"))
            .map_or(out_lines.len(), |i| i + 1);
        out_lines.insert(at, members_line(members));
    }
    let mut out = String::with_capacity(source.len() + 32);
    out.push_str(source.get(..start)?);
    for line in out_lines {
        let _ = writeln!(out, "{line}");
    }
    out.push_str(source.get(end..)?);
    Some(out)
}

/// Adds a member to a list's front matter. Returns `None` when the file
/// has no front matter or the person is already the owner or a member.
#[must_use]
pub fn share(source: &str, member: &str) -> Option<String> {
    let meta = read_meta(source);
    let member = member.trim_start_matches('@').to_ascii_lowercase();
    if meta.owner.as_deref() == Some(member.as_str())
        || meta.members.contains(&member)
    {
        return None;
    }
    let mut members = meta.members;
    members.push(member);
    write_members(source, &members)
}

/// Removes a member. Returns `None` when they were not a member.
#[must_use]
pub fn unshare(source: &str, member: &str) -> Option<String> {
    let meta = read_meta(source);
    let member = member.trim_start_matches('@').to_ascii_lowercase();
    if !meta.members.contains(&member) {
        return None;
    }
    let members: Vec<String> =
        meta.members.into_iter().filter(|m| *m != member).collect();
    write_members(source, &members)
}

/// Reads an "assign to" comment, as in Google Docs.
///
/// A comment containing `assign to @ann` (or `assigned to`), or a `+@ann`
/// mention (what the editor's "Assign to" checkbox writes), becomes a
/// task for those people. Returns the assignees and the task title, or
/// `None` when the comment assigns nobody.
#[must_use]
pub fn comment_assignment(text: &str) -> Option<(Vec<String>, String)> {
    let lower = text.to_lowercase();
    let mut assignees: Vec<String> = Vec::new();
    let mut title = text.to_owned();
    for phrase in ["assign to", "assigned to"] {
        let Some(at) = lower.find(phrase) else {
            continue;
        };
        let after = text.get(at + phrase.len()..).unwrap_or_default();
        let mut consumed = 0;
        for word in after.split_inclusive(char::is_whitespace) {
            let trimmed = word.trim().trim_end_matches([',', '.', ';', ':']);
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("and") {
                consumed += word.len();
                continue;
            }
            if let Some(h) = trimmed.strip_prefix('@').filter(|h| !h.is_empty())
            {
                assignees.push(h.to_ascii_lowercase());
                consumed += word.len();
                continue;
            }
            break;
        }
        if !assignees.is_empty() {
            let end = at + phrase.len() + consumed;
            title = format!(
                "{} {}",
                text.get(..at).unwrap_or_default(),
                text.get(end..).unwrap_or_default()
            );
            break;
        }
    }
    for word in text.split_whitespace() {
        if let Some(h) = word.strip_prefix("+@") {
            let h = h
                .trim_end_matches([',', '.', ';', ':'])
                .to_ascii_lowercase();
            if !h.is_empty() && !assignees.contains(&h) {
                assignees.push(h);
            }
            title = title.replace(word, " ");
        }
    }
    if assignees.is_empty() {
        return None;
    }
    let mut title: String =
        title.split_whitespace().collect::<Vec<_>>().join(" ");
    title = title
        .trim_matches(|c: char| {
            c == '-' || c == ':' || c == ',' || c.is_whitespace()
        })
        .to_owned();
    if title.chars().count() > 140 {
        title = title.chars().take(139).collect::<String>() + "…";
    }
    if title.is_empty() {
        "Follow up on comment".clone_into(&mut title);
    }
    Some((assignees, title))
}

/// The My Work task for an assigning comment.
#[must_use]
pub fn comment_task(
    comment: CommentId,
    doc: DocId,
    text: &str,
    resolved: bool,
) -> Option<Task> {
    let (assignees, title) = comment_assignment(text)?;
    Some(Task {
        source: Source::Comment {
            comment,
            doc,
            resolved,
        },
        title,
        done: resolved,
        fields: TaskFields::default(),
        assignees,
        parent: None,
        list: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id() -> DocId {
        "01M639NFH15EBMG6C7NX1ZK4KA".parse().unwrap()
    }

    #[test]
    fn paths_and_slugs() {
        assert_eq!(slug("Groceries & Errands!"), "groceries-errands");
        assert_eq!(slug("***"), "list");
        assert_eq!(
            list_path("Gourav", "Home Jobs"),
            "tasks/gourav/home-jobs.md"
        );
        assert_eq!(inbox_path("gourav"), "tasks/gourav/inbox.md");
    }

    #[test]
    fn sharing_edits_only_the_members_line() {
        let file = new_list(id(), "Groceries \"weekly\"", "gourav")
            + "- [ ] Milk ^blk-00000001\n";
        let meta = read_meta(&file);
        assert_eq!(meta.title, "Groceries \"weekly\"");
        assert_eq!(meta.owner.as_deref(), Some("gourav"));
        assert_eq!(meta.id, Some(id()));
        let shared = share(&file, "@Ann").unwrap();
        assert!(
            shared.contains(
                "owner: \"@gourav\"\nmembers: [\"@ann\"]\nvellum: 1\n"
            )
        );
        assert!(shared.ends_with("- [ ] Milk ^blk-00000001\n"));
        let shared = share(&shared, "bob").unwrap();
        assert_eq!(read_meta(&shared).members, ["ann", "bob"]);
        assert_eq!(share(&shared, "ann"), None);
        assert_eq!(share(&shared, "gourav"), None);
        let back = unshare(&unshare(&shared, "ann").unwrap(), "bob").unwrap();
        assert_eq!(back, file);
        assert_eq!(unshare(&file, "zed"), None);
        assert_eq!(share("no front matter", "ann"), None);
    }

    #[test]
    fn block_style_members_are_read_and_replaced() {
        let file = "---\ntitle: Trip\nmembers:\n  - \"@ann\"\n  - '@bob'\n\
            vellum: 1\n---\nbody\n";
        assert_eq!(read_meta(file).members, ["ann", "bob"]);
        let out = share(file, "cy").unwrap();
        assert_eq!(
            out,
            "---\ntitle: Trip\nmembers: [\"@ann\", \"@bob\", \"@cy\"]\n\
                vellum: 1\n---\nbody\n"
        );
    }

    #[test]
    fn comments_assign_tasks() {
        let (who, title) = comment_assignment(
            "Can you check the numbers? Assign to @Ann and @bob.",
        )
        .unwrap();
        assert_eq!(who, ["ann", "bob"]);
        assert_eq!(title, "Can you check the numbers?");
        let (who, title) = comment_assignment("+@cy update the chart").unwrap();
        assert_eq!(who, ["cy"]);
        assert_eq!(title, "update the chart");
        assert_eq!(comment_assignment("Looks good @ann"), None);
        let task =
            comment_task(CommentId::generate(), id(), "assign to @ann", true)
                .unwrap();
        assert_eq!(task.title, "Follow up on comment");
        assert!(task.done);
    }
}
