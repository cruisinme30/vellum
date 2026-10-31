// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The in-app inbox: one person's notifications, newest first.
//!
//! A new notification about a subject that already has an unread entry
//! replaces it, so a burst of changes to one issue is one line. The inbox
//! keeps at most [`MAX_ENTRIES`]; the oldest go first.

use jiff::Timestamp;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::notification::{Notification, NotificationKind};

/// The most entries kept per person.
pub const MAX_ENTRIES: usize = 500;

/// One inbox line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxEntry {
    /// A random ID, for marking it read.
    pub id: String,
    /// The kind.
    pub kind: NotificationKind,
    /// What it is about, such as `ENG-142`.
    pub subject: String,
    /// One line.
    pub title: String,
    /// Short body text.
    pub body: String,
    /// Link into the app.
    pub url: String,
    /// Who caused it.
    pub actor: Option<String>,
    /// When it happened.
    pub at: Timestamp,
    /// Whether it was read.
    pub read: bool,
}

/// An inbox line, as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct InboxItem {
    /// Pass it to `notify.read` to mark the line read.
    pub id: String,
    /// The kind, such as `mentioned`.
    pub kind: String,
    /// What it is about, such as `ENG-142`.
    pub subject: String,
    /// One line.
    pub title: String,
    /// Short body text.
    pub body: String,
    /// Link into the app.
    pub url: String,
    /// Who caused it.
    pub actor: Option<String>,
    /// When it happened (RFC 3339).
    pub at: String,
    /// Whether it was read.
    pub read: bool,
}

impl From<&InboxEntry> for InboxItem {
    fn from(e: &InboxEntry) -> Self {
        Self {
            id: e.id.clone(),
            kind: serde_json::to_value(e.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            subject: e.subject.clone(),
            title: e.title.clone(),
            body: e.body.clone(),
            url: e.url.clone(),
            actor: e.actor.clone(),
            at: e.at.to_string(),
            read: e.read,
        }
    }
}

/// Adds `notification` to the front of `inbox` under `id`.
pub fn add(
    inbox: &mut Vec<InboxEntry>,
    notification: &Notification,
    id: String,
) {
    inbox.retain(|e| e.read || e.subject != notification.subject);
    inbox.insert(
        0,
        InboxEntry {
            id,
            kind: notification.kind,
            subject: notification.subject.clone(),
            title: notification.title.clone(),
            body: notification.body.clone(),
            url: notification.url.clone(),
            actor: notification.actor.clone(),
            at: notification.at,
            read: false,
        },
    );
    inbox.truncate(MAX_ENTRIES);
}

/// Marks entries read: the listed IDs, or all of them. Returns how many
/// changed.
pub fn mark_read(inbox: &mut [InboxEntry], ids: &[String], all: bool) -> usize {
    let mut changed = 0;
    for entry in inbox.iter_mut().filter(|e| !e.read) {
        if all || ids.contains(&entry.id) {
            entry.read = true;
            changed += 1;
        }
    }
    changed
}

/// Whether an unread entry about `subject` exists (email waits on this).
#[must_use]
pub fn unread_about(inbox: &[InboxEntry], subject: &str) -> bool {
    inbox.iter().any(|e| !e.read && e.subject == subject)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(subject: &str, title: &str, minute: i64) -> Notification {
        Notification {
            kind: NotificationKind::Commented,
            subject: subject.to_owned(),
            title: title.to_owned(),
            body: String::new(),
            url: format!("/issue/{subject}"),
            actor: Some("ada".to_owned()),
            at: Timestamp::from_second(minute * 60)
                .unwrap_or(Timestamp::UNIX_EPOCH),
        }
    }

    #[test]
    fn unread_lines_about_one_subject_collapse() {
        let mut inbox = Vec::new();
        add(&mut inbox, &note("ENG-1", "first", 0), "a".to_owned());
        add(&mut inbox, &note("ENG-2", "other", 1), "b".to_owned());
        add(&mut inbox, &note("ENG-1", "second", 2), "c".to_owned());
        let titles: Vec<_> = inbox.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["second", "other"]);

        // A read line stays as history.
        assert_eq!(mark_read(&mut inbox, &["c".to_owned()], false), 1);
        assert!(!unread_about(&inbox, "ENG-1"));
        add(&mut inbox, &note("ENG-1", "third", 3), "d".to_owned());
        assert_eq!(inbox.len(), 3);
        assert!(unread_about(&inbox, "ENG-1"));
        assert_eq!(mark_read(&mut inbox, &[], true), 2);
    }

    #[test]
    fn the_inbox_is_bounded() {
        let mut inbox = Vec::new();
        for n in 0..(MAX_ENTRIES + 10) {
            add(
                &mut inbox,
                &note(&format!("ENG-{n}"), "t", 0),
                n.to_string(),
            );
        }
        assert_eq!(inbox.len(), MAX_ENTRIES);
        assert_eq!(
            inbox.first().map(|e| e.id.as_str()),
            Some((MAX_ENTRIES + 9).to_string().as_str())
        );
    }

    #[test]
    fn the_api_names_kinds_like_preferences_do() {
        let mut inbox = Vec::new();
        add(&mut inbox, &note("ENG-1", "t", 0), "a".to_owned());
        let item = InboxItem::from(&inbox[0]);
        assert_eq!(item.kind, "commented");
    }
}
