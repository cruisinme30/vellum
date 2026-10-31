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
use serde_json::{Map, Value, json};
use vellum_work::comment::Target;
use vellum_work::event::IssueSnapshot;
use vellum_work::{IssueId, UserRef, WorkEvent};

use crate::event::{EventAction, EventType, WebhookEvent};
use crate::notification::{Notification, NotificationKind};

/// What a comment's issue looks like now, for its subject and recipients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueInfo {
    /// The identifier, such as `ENG-142`.
    pub key: String,
    /// The title.
    pub title: String,
    /// The team key, such as `ENG`.
    pub team: String,
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
pub fn notifications(
    event: &WorkEvent,
    lookup: &dyn IssueLookup,
    at: Timestamp,
) -> Vec<Addressed> {
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
                .map(|i| {
                    i.subscribers.iter().map(ToString::to_string).collect()
                })
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
                vellum_work::sla::Risk::Gray
                | vellum_work::sla::Risk::Yellow => return out,
            };
            let title = if kind == NotificationKind::SlaBreached {
                format!("{} breached its SLA", issue.key)
            } else {
                format!("{} is close to breaching its SLA", issue.key)
            };
            out.push(Addressed {
                notification: note(
                    kind,
                    issue,
                    title,
                    issue.title.clone(),
                    None,
                    at,
                ),
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
                out.extend(comment_notes(
                    &info, comment, mentions, recipients, at,
                ));
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
        url: format!("/issues/{}", info.key),
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

fn assigned(
    issue: &IssueSnapshot,
    assignee: &UserRef,
    at: Timestamp,
) -> Addressed {
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
        url: format!("/issues/{}", issue.key),
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

/// The webhook events `event` causes (#149, D176).
///
/// | Work event | Webhook |
/// |---|---|
/// | issue created | `issue.create` |
/// | issue updated, status changed, moved, triaged | `issue.update` with `updated_from` |
/// | cycle created / rolled over | `cycle.create` / `cycle.update` |
/// | SLA started / orange / red | `issue_sla.sla_set` / `.sla_high_risk` / `.sla_breached` |
/// | project created / changed | `project.create` / `project.update` |
/// | project update posted | `project_update.create` |
/// | label created, updated, deleted | `label.create` / `.update` / `.remove` |
/// | comment on an issue added, edited, deleted | `comment.create` / `.update` / `.remove` |
///
/// Comments on documents are never sent: a document's readers are set by
/// its own permissions, which a webhook cannot check.
#[must_use]
#[expect(clippy::too_many_lines, reason = "one arm per Work event")]
pub fn webhook_events(
    event: &WorkEvent,
    lookup: &dyn IssueLookup,
    at: Timestamp,
) -> Vec<WebhookEvent> {
    let issue_url = |key: &str| Some(format!("/issues/{key}"));
    let snapshot = |issue: &IssueSnapshot| {
        serde_json::to_value(issue).unwrap_or(Value::Null)
    };
    let make = |event_type,
                action,
                team: Option<String>,
                data: Value,
                from: Option<Value>,
                url| {
        WebhookEvent {
            id: uuid::Uuid::now_v7().to_string(),
            event_type,
            action,
            occurred_at: at,
            team,
            actor: None,
            data,
            updated_from: from,
            url,
        }
    };
    let issue_event = |issue: &IssueSnapshot, action, from: Option<Value>| {
        make(
            EventType::Issue,
            action,
            Some(issue.team.to_string()),
            snapshot(issue),
            from,
            issue_url(&issue.key.to_string()),
        )
    };
    let comment = |thread: &str,
                   target: &Target,
                   comment: Option<&vellum_work::comment::Comment>,
                   id: &str,
                   action| {
        let Target::Issue { issue } = target else {
            return None;
        };
        let info = lookup.issue(*issue)?;
        let mut data = json!({
            "id": id,
            "thread": thread,
            "issue": info.key,
        });
        if let (Some(c), Some(map)) = (comment, data.as_object_mut()) {
            let _ =
                map.insert("author".to_owned(), json!(c.author.to_string()));
            let _ = map.insert("body".to_owned(), json!(c.body));
            let _ = map.insert(
                "created_at".to_owned(),
                serde_json::to_value(c)
                    .ok()
                    .and_then(|v| v.get("created_at").cloned())
                    .unwrap_or(Value::Null),
            );
        }
        Some(make(
            EventType::Comment,
            action,
            Some(info.team),
            data,
            None,
            issue_url(&info.key),
        ))
    };
    let out = match event {
        WorkEvent::IssueCreated { issue, .. } => {
            Some(issue_event(issue, EventAction::Create, None))
        }
        WorkEvent::IssueUpdated { issue, changes } => {
            let from: Map<String, Value> = changes
                .iter()
                .map(|c| {
                    (
                        c.field.clone(),
                        c.from.clone().map_or(Value::Null, Value::String),
                    )
                })
                .collect();
            Some(issue_event(
                issue,
                EventAction::Update,
                Some(Value::Object(from)),
            ))
        }
        WorkEvent::IssueStateChanged { issue, from, .. } => Some(issue_event(
            issue,
            EventAction::Update,
            Some(json!({ "state": from.to_string() })),
        )),
        WorkEvent::IssueMoved { issue, from } => Some(issue_event(
            issue,
            EventAction::Update,
            Some(json!({ "key": from.to_string() })),
        )),
        WorkEvent::IssueTriaged { issue, action } => {
            let mut e = issue_event(
                issue,
                EventAction::Update,
                Some(json!({ "triage": null })),
            );
            if let Some(map) = e.data.as_object_mut() {
                let _ = map.insert("triage".to_owned(), json!(action));
            }
            Some(e)
        }
        WorkEvent::CycleCreated { team, cycle } => Some(make(
            EventType::Cycle,
            EventAction::Create,
            Some(team.to_string()),
            json!({ "team": team.to_string(), "cycle": cycle.to_string() }),
            None,
            None,
        )),
        WorkEvent::CycleRolledOver {
            team,
            from,
            to,
            moved,
            removed,
        } => Some(make(
            EventType::Cycle,
            EventAction::Update,
            Some(team.to_string()),
            json!({
                "team": team.to_string(),
                "cycle": from.to_string(),
                "rolled_over_to": to.as_ref().map(ToString::to_string),
                "moved": moved.len(),
                "removed": removed.len(),
            }),
            None,
            None,
        )),
        WorkEvent::SlaStarted {
            issue,
            kind,
            breaches_at,
        } => {
            let mut data = snapshot(issue);
            if let Some(map) = data.as_object_mut() {
                let _ = map.insert("sla".to_owned(), json!(kind));
                let _ = map.insert(
                    "breaches_at".to_owned(),
                    json!(breaches_at.unix_timestamp()),
                );
            }
            Some(make(
                EventType::IssueSla,
                EventAction::SlaSet,
                Some(issue.team.to_string()),
                data,
                None,
                issue_url(&issue.key.to_string()),
            ))
        }
        WorkEvent::SlaRisk { issue, risk, .. } => {
            let action = match risk {
                vellum_work::sla::Risk::Orange => {
                    Some(EventAction::SlaHighRisk)
                }
                vellum_work::sla::Risk::Red => Some(EventAction::SlaBreached),
                _ => None,
            };
            action.map(|action| {
                make(
                    EventType::IssueSla,
                    action,
                    Some(issue.team.to_string()),
                    snapshot(issue),
                    None,
                    issue_url(&issue.key.to_string()),
                )
            })
        }
        WorkEvent::ProjectCreated { project } => Some(make(
            EventType::Project,
            EventAction::Create,
            None,
            json!({ "id": project.to_string() }),
            None,
            None,
        )),
        WorkEvent::ProjectChanged { project, changes } => Some(make(
            EventType::Project,
            EventAction::Update,
            None,
            json!({ "id": project.to_string(), "changes": changes }),
            None,
            None,
        )),
        WorkEvent::ProjectUpdatePosted {
            project,
            update,
            health,
        } => Some(make(
            EventType::ProjectUpdate,
            EventAction::Create,
            None,
            json!({
                "id": update.to_string(),
                "project": project.to_string(),
                "health": serde_json::to_value(health).unwrap_or(Value::Null),
            }),
            None,
            None,
        )),
        WorkEvent::LabelChanged {
            team,
            label,
            change,
        } => {
            let action = match change.as_str() {
                "created" => EventAction::Create,
                "deleted" => EventAction::Remove,
                _ => EventAction::Update,
            };
            Some(make(
                EventType::Label,
                action,
                team.as_ref().map(ToString::to_string),
                json!({
                    "label": label,
                    "team": team.as_ref().map(ToString::to_string)
                }),
                None,
                None,
            ))
        }
        WorkEvent::CommentAdded {
            thread,
            target,
            comment: c,
            ..
        } => comment(
            &thread.to_string(),
            target,
            Some(c),
            &c.id.to_string(),
            EventAction::Create,
        ),
        WorkEvent::CommentEdited {
            thread,
            target,
            comment: c,
            ..
        } => comment(
            &thread.to_string(),
            target,
            Some(c),
            &c.id.to_string(),
            EventAction::Update,
        ),
        WorkEvent::CommentDeleted {
            thread,
            target,
            comment: id,
            ..
        } => comment(
            &thread.to_string(),
            target,
            None,
            &id.to_string(),
            EventAction::Remove,
        ),
        _ => None,
    };
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

    use std::collections::BTreeMap;

    use time::macros::datetime;
    use vellum_work::{
        Comments, Ctx, IssuePatch, NewComment, NewIssue, Team, TeamKey,
        WorkGraph,
    };

    use super::*;

    struct Graph<'a>(&'a WorkGraph);

    impl IssueLookup for Graph<'_> {
        fn issue(&self, id: IssueId) -> Option<IssueInfo> {
            self.0.issue(&id).map(|i| IssueInfo {
                key: i.key().to_string(),
                title: i.title.clone(),
                team: i.team.to_string(),
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
    fn sent(
        graph: &WorkGraph,
        events: &[WorkEvent],
    ) -> BTreeMap<NotificationKind, Vec<String>> {
        events
            .iter()
            .flat_map(|e| {
                notifications(e, &Graph(graph), Timestamp::UNIX_EPOCH)
            })
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
            sent(&graph, &reassigned.events)
                .get(&NotificationKind::IssueAssigned),
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
            .flat_map(|e| {
                notifications(e, &Graph(&graph), Timestamp::UNIX_EPOCH)
            })
            .collect();
        let mention = notes
            .iter()
            .find(|a| a.notification.kind == NotificationKind::Mentioned)
            .unwrap();
        assert_eq!(mention.to, ["dee"]);
        assert_eq!(mention.notification.subject, "ENG-1");
        assert_eq!(mention.notification.url, "/issues/ENG-1");
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
    fn work_events_become_webhook_events_and_doc_comments_stay_inside() {
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
        let created = graph
            .create_issue(NewIssue::new(id, eng, "Fix login"), &ctx("ada"))
            .unwrap();
        let hooks: Vec<WebhookEvent> = created
            .events
            .iter()
            .flat_map(|e| {
                webhook_events(e, &Graph(&graph), Timestamp::UNIX_EPOCH)
            })
            .collect();
        let issue = hooks
            .iter()
            .find(|e| e.event_type == EventType::Issue)
            .unwrap();
        assert_eq!(issue.action, EventAction::Create);
        assert_eq!(issue.team.as_deref(), Some("ENG"));
        assert_eq!(issue.data["key"], "ENG-1");
        assert_eq!(issue.url.as_deref(), Some("/issues/ENG-1"));
        assert!(hooks.iter().all(|e| e.validate().is_ok()));

        let patch = IssuePatch {
            title: Some("Fix login redirect".to_owned()),
            ..IssuePatch::default()
        };
        let updated = graph.update_issue(&id, patch, &ctx("ada")).unwrap();
        let update = updated
            .events
            .iter()
            .flat_map(|e| {
                webhook_events(e, &Graph(&graph), Timestamp::UNIX_EPOCH)
            })
            .find(|e| e.event_type == EventType::Issue)
            .unwrap();
        assert_eq!(update.action, EventAction::Update);
        assert_eq!(update.updated_from.unwrap()["title"], "Fix login");

        let mut comments = Comments::new();
        let on_issue = comments
            .add(
                NewComment::new(
                    vellum_kernel::CommentId::generate(),
                    Target::Issue { issue: id },
                    "Looks good",
                ),
                &[],
                &ctx("bo"),
            )
            .unwrap();
        let sent: Vec<WebhookEvent> = on_issue
            .events
            .iter()
            .flat_map(|e| {
                webhook_events(e, &Graph(&graph), Timestamp::UNIX_EPOCH)
            })
            .collect();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].event_type, EventType::Comment);
        assert_eq!(sent[0].data["issue"], "ENG-1");
        assert_eq!(sent[0].data["body"], "Looks good");

        let on_doc = comments
            .add(
                NewComment::new(
                    vellum_kernel::CommentId::generate(),
                    Target::Doc {
                        doc: vellum_kernel::DocId::generate(),
                        block: None,
                    },
                    "Private note",
                ),
                &[],
                &ctx("bo"),
            )
            .unwrap();
        assert!(
            on_doc
                .events
                .iter()
                .flat_map(|e| webhook_events(
                    e,
                    &Graph(&graph),
                    Timestamp::UNIX_EPOCH
                ))
                .next()
                .is_none()
        );
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
