// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Which issue-tracker events notify whom.
//!
//! | Event | Kind | Who |
//! |---|---|---|
//! | issue created or updated with a new assignee | `issue_assigned` | the assignee |
//! | issue status changed | `state_changed` | subscribers and the assignee |
//! | comment posted | `mentioned` | people mentioned in it |
//! | comment posted | `commented` | everyone else the issue core lists |
//! | SLA risk turned orange | `sla_high_risk` | the people the SLA names |
//! | SLA risk turned red | `sla_breached` | the people the SLA names |
//!
//! People are named by handle here; the caller maps handles to accounts.
//! The issue core leaves the comment author out of comment recipients; the
//! planner ([`crate::notification::plan`]) leaves out the actor of the
//! other events when it is known.

use jiff::Timestamp;
use vellum_work::comment::Target;
use vellum_work::event::IssueSnapshot;
use vellum_work::{IssueId, UserRef, WorkEvent};

use crate::notification::{Notification, NotificationKind};

/// What a comment's issue looks like now, for its subject and recipients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueInfo {
    /// The identifier, such as `ENG-142`.
    pub key: String,
    /// The title.
    pub title: String,
    /// People who follow the issue.
    pub subscribers: Vec<UserRef>,
}

/// Looks issues up while mapping events.
pub trait IssueLookup {
    /// The issue with `id`, if it still exists.
    fn issue(&self, id: IssueId) -> Option<IssueInfo>;
}

/// One notification and the handles it goes to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Addressed {
    /// What to say.
    pub notification: Notification,
    /// Who to say it to, by handle.
    pub to: Vec<String>,
}

/// The notifications `event` causes.
#[must_use]
pub fn notifications(event: &WorkEvent, lookup: &dyn IssueLookup, at: Timestamp) -> Vec<Addressed> {
    let mut out = Vec::new();
    match event {
        WorkEvent::IssueCreated { issue, .. } => {
            if let Some(assignee) = &issue.assignee {
                out.push(assigned(issue, assignee, at));
            }
        }
        WorkEvent::IssueUpdated { issue, changes } => {
            let reassigned = changes
                .iter()
                .any(|c| c.field == "assignee" && c.to.is_some());
            if reassigned && let Some(assignee) = &issue.assignee {
                out.push(assigned(issue, assignee, at));
            }
        }
        WorkEvent::IssueStateChanged { issue, .. } => {
            let mut to: Vec<String> = lookup
                .issue(issue.id)
                .map(|i| i.subscribers.iter().map(ToString::to_string).collect())
                .unwrap_or_default();
            to.extend(issue.assignee.iter().map(ToString::to_string));
            to.sort();
            to.dedup();
            out.push(Addressed {
                notification: note(
                    NotificationKind::StateChanged,
                    issue,
                    format!("{} moved to {}", issue.key, issue.state),
                    issue.title.clone(),
                    None,
                    at,
                ),
                to,
            });
        }
        WorkEvent::SlaRisk {
            issue,
            risk,
            recipients,
        } => {
            let kind = match risk {
                vellum_work::sla::Risk::Red => NotificationKind::SlaBreached,
                vellum_work::sla::Risk::Orange => NotificationKind::SlaHighRisk,
                vellum_work::sla::Risk::Gray | vellum_work::sla::Risk::Yellow => return out,
            };
            let title = if kind == NotificationKind::SlaBreached {
                format!("{} breached its SLA", issue.key)
            } else {
                format!("{} is close to breaching its SLA", issue.key)
            };
            out.push(Addressed {
                notification: note(kind, issue, title, issue.title.clone(), None, at),
                to: recipients.iter().map(ToString::to_string).collect(),
            });
        }
        WorkEvent::CommentAdded {
            target: Target::Issue { issue },
            comment,
            mentions,
            recipients,
            ..
        } => {
            if let Some(info) = lookup.issue(*issue) {
                out.extend(comment_notes(&info, comment, mentions, recipients, at));
            }
        }
        _ => {}
    }
    out
}

/// A comment's notifications: `mentioned` for the people it names,
/// `commented` for everyone else listed, never the author.
fn comment_notes(
    info: &IssueInfo,
    comment: &vellum_work::Comment,
    mentions: &[UserRef],
    recipients: &[UserRef],
    at: Timestamp,
) -> Vec<Addressed> {
    let mut out = Vec::new();
    let author = comment.author.to_string();
    let body = excerpt(&comment.body);
    let mentioned: Vec<String> = mentions
        .iter()
        .map(ToString::to_string)
        .filter(|h| *h != author)
        .collect();
    let comment_note = |kind, title: String| Notification {
        kind,
        subject: info.key.clone(),
        title,
        body: body.clone(),
        url: format!("/issue/{}", info.key),
        actor: Some(author.clone()),
        at,
    };
    if !mentioned.is_empty() {
        out.push(Addressed {
            notification: comment_note(
                NotificationKind::Mentioned,
                format!("{author} mentioned you on {}", info.key),
            ),
            to: mentioned.clone(),
        });
    }
    let others: Vec<String> = recipients
        .iter()
        .map(ToString::to_string)
        .filter(|h| *h != author && !mentioned.contains(h))
        .collect();
    if !others.is_empty() {
        out.push(Addressed {
            notification: comment_note(
                NotificationKind::Commented,
                format!("{author} commented on {}: {}", info.key, info.title),
            ),
            to: others,
        });
    }
    out
}

fn assigned(issue: &IssueSnapshot, assignee: &UserRef, at: Timestamp) -> Addressed {
    Addressed {
        notification: note(
            NotificationKind::IssueAssigned,
            issue,
            format!("{} was assigned to you", issue.key),
            issue.title.clone(),
            None,
            at,
        ),
        to: vec![assignee.to_string()],
    }
}

fn note(
    kind: NotificationKind,
    issue: &IssueSnapshot,
    title: String,
    body: String,
    actor: Option<String>,
    at: Timestamp,
) -> Notification {
    Notification {
        kind,
        subject: issue.key.to_string(),
        title,
        body,
        url: format!("/issue/{}", issue.key),
        actor,
        at,
    }
}

/// The first 140 characters of a comment, on one line.
fn excerpt(body: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 140 {
        return flat;
    }
    let mut short: String = flat.chars().take(139).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

    use std::collections::BTreeMap;

    use time::macros::datetime;
    use vellum_work::{Comments, Ctx, IssuePatch, NewComment, NewIssue, Team, TeamKey, WorkGraph};

    use super::*;

    struct Graph<'a>(&'a WorkGraph);

    impl IssueLookup for Graph<'_> {
        fn issue(&self, id: IssueId) -> Option<IssueInfo> {
            self.0.issue(&id).map(|i| IssueInfo {
                key: i.key().to_string(),
                title: i.title.clone(),
                subscribers: i.subscribers.clone(),
            })
        }
    }

    fn ctx(actor: &str) -> Ctx {
        let now = datetime!(2026-11-02 09:00 UTC);
        Ctx {
            now,
            today: now.date(),
            actor: Some(UserRef::parse(actor).unwrap()),
        }
    }

    fn user(handle: &str) -> UserRef {
        UserRef::parse(handle).unwrap()
    }

    /// Kind to recipients, for every notification of `events`.
    fn sent(graph: &WorkGraph, events: &[WorkEvent]) -> BTreeMap<NotificationKind, Vec<String>> {
        events
            .iter()
            .flat_map(|e| notifications(e, &Graph(graph), Timestamp::UNIX_EPOCH))
            .map(|a| (a.notification.kind, a.to))
            .collect()
    }

    #[test]
    fn assignments_state_changes_and_comments_reach_the_right_people() {
        let mut graph = WorkGraph::new();
        let eng = TeamKey::parse("ENG").unwrap();
        graph
            .insert_team(
                Team::new(
                    "01JC00000000000000000000E1".parse().unwrap(),
                    eng.clone(),
                    "Engineering",
                )
                .unwrap(),
            )
            .unwrap();
        let id = IssueId::generate();
        let mut draft = NewIssue::new(id, eng, "Fix login");
        draft.assignee = Some(user("bo"));
        let created = graph.create_issue(draft, &ctx("ada")).unwrap();
        let by_kind = sent(&graph, &created.events);
        assert_eq!(
            by_kind.get(&NotificationKind::IssueAssigned),
            Some(&vec!["bo".to_owned()])
        );

        let patch = IssuePatch {
            assignee: Some(Some(user("cy"))),
            ..IssuePatch::default()
        };
        let reassigned = graph.update_issue(&id, patch, &ctx("ada")).unwrap();
        assert_eq!(
            sent(&graph, &reassigned.events).get(&NotificationKind::IssueAssigned),
            Some(&vec!["cy".to_owned()])
        );

        let mut comments = Comments::new();
        let audience = graph.issue(&id).unwrap().subscribers.clone();
        let draft = NewComment::new(
            vellum_kernel::CommentId::generate(),
            Target::Issue { issue: id },
            "@dee can you look? This **breaks** sign-in for everyone.",
        );
        let added = comments.add(draft, &audience, &ctx("cy")).unwrap();
        let notes: Vec<Addressed> = added
            .events
            .iter()
            .flat_map(|e| notifications(e, &Graph(&graph), Timestamp::UNIX_EPOCH))
            .collect();
        let mention = notes
            .iter()
            .find(|a| a.notification.kind == NotificationKind::Mentioned)
            .unwrap();
        assert_eq!(mention.to, ["dee"]);
        assert_eq!(mention.notification.subject, "ENG-1");
        assert_eq!(mention.notification.url, "/issue/ENG-1");
        assert_eq!(mention.notification.actor.as_deref(), Some("cy"));
        // The author never hears about their own comment, and the
        // mentioned person gets one line, not two.
        for a in &notes {
            assert!(!a.to.contains(&"cy".to_owned()), "{a:?}");
            if a.notification.kind == NotificationKind::Commented {
                assert!(!a.to.contains(&"dee".to_owned()), "{a:?}");
            }
        }
        let _ = graph.apply_comment(&added, &ctx("cy")).unwrap();

        let done = graph
            .set_state(
                &id,
                vellum_work::workflow::StatusKey::parse("done").unwrap(),
                &ctx("cy"),
            )
            .unwrap();
        let moved = sent(&graph, &done.events);
        let to = moved.get(&NotificationKind::StateChanged).unwrap();
        assert!(to.contains(&"cy".to_owned()), "{to:?}");
    }

    #[test]
    fn long_comments_are_cut_to_one_line() {
        let text = format!("first line\n\n{}", "word ".repeat(100));
        let short = excerpt(&text);
        assert!(!short.contains('\n'));
        assert_eq!(short.chars().count(), 140);
        assert!(short.ends_with('…'));
        assert_eq!(excerpt("  hi \n there "), "hi there");
    }
}
