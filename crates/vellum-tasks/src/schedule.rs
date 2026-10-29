// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Sending reminders and the morning digest (R-MW-5, #472, D180).
//!
//! [`TasksService::notices`] says what one person should hear about in a
//! window: each reminder that fires in it and, if their digest time falls
//! in it, the digest. [`Schedule`] holds what is planned for the next few
//! minutes and hands out what is due, so the server reads each person's
//! tasks every few minutes rather than every few seconds:
//!
//! - every [`PLAN_EVERY`] everyone is planned up to [`LOOKAHEAD`] ahead;
//! - between those, people whose tasks changed are planned again, so a
//!   reminder set for two minutes from now is not missed;
//! - [`Schedule::fire`] returns what is due, in time order, and moves the
//!   watermark to now. Nothing at or before the watermark is planned or
//!   sent again.
//!
//! Delivery (inbox, web push, e-mail) is the notifications service's job.

use std::collections::HashMap;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use vellum_kernel::{Error, UserId};

use crate::model::{Due, Source};
use crate::projection;
use crate::remind;
use crate::service::TasksService;

/// How often everyone's reminders are planned again.
pub const PLAN_EVERY: Duration = Duration::from_mins(5);

/// How far ahead a plan reaches; a little more than [`PLAN_EVERY`] so
/// plans overlap.
pub const LOOKAHEAD: SignedDuration = SignedDuration::from_mins(6);

/// How often the scheduler checks for due notices.
pub const TICK: Duration = Duration::from_secs(20);

/// The furthest back a restarted server catches up.
pub const CATCH_UP: SignedDuration = SignedDuration::from_mins(15);

/// What a notice is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
    /// A task reminder.
    Reminder,
    /// The morning digest.
    Digest,
}

/// Something one person should be told at a given time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Who.
    pub user: UserId,
    /// What kind.
    pub kind: NoticeKind,
    /// What it is about: the task key, or `digest`.
    pub subject: String,
    /// One line.
    pub title: String,
    /// Short body text.
    pub body: String,
    /// Where it links in the app.
    pub url: String,
    /// When it is due.
    pub at: Timestamp,
}

fn task_url(source: &Source) -> String {
    match source {
        Source::Checklist { doc, .. }
        | Source::Comment { doc, .. }
        | Source::Mention { doc: Some(doc), .. } => format!("/p/{doc}"),
        Source::Issue { key, .. }
        | Source::Mention {
            issue: Some(key), ..
        } => format!("/issues/{key}"),
        Source::Mention { .. } | Source::Review { .. } => "/my-work".to_owned(),
    }
}

impl TasksService {
    /// Everyone whose reminders the server sends.
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub async fn people(&self) -> Result<Vec<UserId>, Error> {
        self.workspace.people().await
    }

    /// What `user` should be told in `(after, until]`: due reminders and,
    /// when their digest time falls in the window and the day is not
    /// clear, the digest.
    ///
    /// # Errors
    ///
    /// Fails when the workspace does.
    pub async fn notices(
        &self,
        user: UserId,
        after: Timestamp,
        until: Timestamp,
    ) -> Result<Vec<Notice>, Error> {
        let profile = self.workspace.profile(user).await?;
        let settings = profile.preferences.reminders(profile.time_zone.clone());
        let tasks = self.gather(user, &profile.handle).await?;
        let mut out: Vec<Notice> = Vec::new();
        for due in remind::due_between(&tasks, after, until, &settings) {
            let Some(task) = tasks.iter().find(|t| t.source.key() == due.task) else {
                continue;
            };
            let body = match task.fields.due {
                Some(Due::DateTime(moment)) => {
                    format!("Due {}", moment.strftime("%a %-d %b %H:%M"))
                }
                Some(Due::Date(day)) => format!("Due {}", day.strftime("%a %-d %b")),
                None => String::new(),
            };
            out.push(Notice {
                user,
                kind: NoticeKind::Reminder,
                subject: format!("{}@{}", due.task, due.at.as_second()),
                title: due.title,
                body,
                url: task_url(&task.source),
                at: due.at,
            });
        }
        let from = after.to_zoned(settings.time_zone.clone());
        if let Some(next) = remind::next_digest(&from, &settings)
            && next.timestamp() <= until
        {
            let view = projection::build(tasks, next.date());
            if let Some(markdown) = remind::digest(&view, next.date()) {
                let today = view.today.len() + view.evening.len();
                out.push(Notice {
                    user,
                    kind: NoticeKind::Digest,
                    subject: format!("digest.{}", next.date()),
                    title: format!("Your day: {today} to do, {} in the Inbox", view.inbox.len()),
                    // The counts line; the whole digest is in My Work.
                    body: markdown
                        .lines()
                        .find(|l| !l.is_empty() && !l.starts_with('#'))
                        .unwrap_or_default()
                        .to_owned(),
                    url: "/my-work".to_owned(),
                    at: next.timestamp(),
                });
            }
        }
        out.sort_by_key(|n| n.at);
        Ok(out)
    }
}

/// What is planned for the next few minutes, per person.
#[derive(Debug, Clone)]
pub struct Schedule {
    planned: HashMap<UserId, Vec<Notice>>,
    watermark: Timestamp,
    horizon: Timestamp,
}

impl Schedule {
    /// An empty schedule that sends nothing due at or before `watermark`.
    #[must_use]
    pub fn new(watermark: Timestamp) -> Self {
        Self {
            planned: HashMap::new(),
            watermark,
            horizon: watermark,
        }
    }

    /// Nothing due at or before this is sent.
    #[must_use]
    pub const fn watermark(&self) -> Timestamp {
        self.watermark
    }

    /// How far plans reach.
    #[must_use]
    pub const fn horizon(&self) -> Timestamp {
        self.horizon
    }

    /// The window a full plan at `now` covers, which then becomes the
    /// horizon.
    pub fn extend(&mut self, now: Timestamp) -> (Timestamp, Timestamp) {
        let until = now
            .saturating_add(LOOKAHEAD)
            .unwrap_or(now)
            .max(self.horizon);
        self.horizon = until;
        (self.watermark, until)
    }

    /// Replaces what is planned for `user`.
    pub fn plan(&mut self, user: UserId, notices: Vec<Notice>) {
        let watermark = self.watermark;
        let horizon = self.horizon;
        let keep: Vec<Notice> = notices
            .into_iter()
            .filter(|n| n.user == user && n.at > watermark && n.at <= horizon)
            .collect();
        if keep.is_empty() {
            let _ = self.planned.remove(&user);
        } else {
            let _ = self.planned.insert(user, keep);
        }
    }

    /// Takes everything due by `now`, oldest first, and moves the
    /// watermark to `now`.
    pub fn fire(&mut self, now: Timestamp) -> Vec<Notice> {
        if now <= self.watermark {
            return Vec::new();
        }
        let mut due = Vec::new();
        self.planned.retain(|_, notices| {
            let (now_due, later): (Vec<Notice>, Vec<Notice>) =
                notices.drain(..).partition(|n| n.at <= now);
            due.extend(now_due);
            *notices = later;
            !notices.is_empty()
        });
        due.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.subject.cmp(&b.subject)));
        self.watermark = now;
        self.horizon = self.horizon.max(now);
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minute: i64) -> Timestamp {
        Timestamp::from_second(1_800_000_000 + minute * 60).unwrap()
    }

    fn notice(user: UserId, minute: i64, subject: &str) -> Notice {
        Notice {
            user,
            kind: NoticeKind::Reminder,
            subject: subject.to_owned(),
            title: subject.to_owned(),
            body: String::new(),
            url: "/my-work".to_owned(),
            at: at(minute),
        }
    }

    #[test]
    fn notices_fire_once_in_time_order() {
        let (ana, ben) = (UserId::generate(), UserId::generate());
        let mut s = Schedule::new(at(0));
        assert_eq!(s.extend(at(0)), (at(0), at(6)));
        s.plan(ana, vec![notice(ana, 3, "b"), notice(ana, 1, "a")]);
        s.plan(ben, vec![notice(ben, 2, "c"), notice(ben, 9, "too far")]);
        let fired: Vec<String> = s.fire(at(2)).into_iter().map(|n| n.subject).collect();
        assert_eq!(fired, ["a", "c"]);
        assert!(s.fire(at(2)).is_empty());
        // A later plan cannot bring back what was sent.
        s.plan(ana, vec![notice(ana, 1, "a"), notice(ana, 3, "b")]);
        let fired: Vec<String> = s.fire(at(4)).into_iter().map(|n| n.subject).collect();
        assert_eq!(fired, ["b"]);
        assert_eq!(s.watermark(), at(4));
    }

    #[test]
    fn a_new_plan_replaces_the_old_one() {
        let ana = UserId::generate();
        let mut s = Schedule::new(at(0));
        let _ = s.extend(at(0));
        s.plan(ana, vec![notice(ana, 3, "done since")]);
        s.plan(ana, vec![]);
        assert!(s.fire(at(5)).is_empty());
    }

    #[test]
    fn plans_reach_from_the_watermark_to_the_lookahead() {
        let mut s = Schedule::new(at(0));
        let _ = s.extend(at(0));
        let _ = s.fire(at(5));
        assert_eq!(s.extend(at(5)), (at(5), at(11)));
        // The horizon never moves back.
        assert_eq!(s.extend(at(1)), (at(5), at(11)));
        assert_eq!(s.horizon(), at(11));
    }

    #[test]
    fn notices_for_someone_else_are_ignored() {
        let (ana, ben) = (UserId::generate(), UserId::generate());
        let mut s = Schedule::new(at(0));
        let _ = s.extend(at(0));
        s.plan(ana, vec![notice(ben, 1, "not ana's")]);
        assert!(s.fire(at(2)).is_empty());
    }
}
