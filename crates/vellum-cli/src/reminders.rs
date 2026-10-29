// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Sends My Work reminders and morning digests (R-MW-5, #472, D180).
//!
//! One replica at a time holds the `tasks.reminders` cluster lease
//! ([`Replica::every`]) and runs the [`Schedule`]: it plans everyone every few minutes, plans again anyone
//! whose tasks changed, and hands what falls due to the notifications
//! [`Dispatcher`] (inbox line, web push, e-mail by each person's
//! preferences). The time it has sent up to is kept in
//! `<data>/.vellum/tasks/reminders.json`, so a restart neither repeats
//! nor drops a reminder; a restart catches up at most
//! [`CATCH_UP`].

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use vellum_kernel::{Error, ErrorKind};
use vellum_notify::delivery::HttpTransport;
use vellum_notify::dispatch::Dispatcher;
use vellum_notify::endpoint::{SafeTransport, policy};
use vellum_notify::notification::{Notification, NotificationKind};
use vellum_notify::service::NotifyStore;
use vellum_tasks::schedule::{CATCH_UP, Notice, NoticeKind, PLAN_EVERY, Schedule, TICK};
use vellum_tasks::service::TasksService;

use crate::cluster::Replica;

/// The lease that makes one replica the sender.
pub const LEASE: &str = "tasks.reminders";

#[derive(Debug, Serialize, Deserialize)]
struct Mark {
    sent_until: Timestamp,
}

/// The file that remembers how far reminders were sent.
#[derive(Debug, Clone)]
pub struct Watermark {
    path: Option<PathBuf>,
}

impl Watermark {
    /// `<dir>/reminders.json`, or nothing kept when `dir` is `None`.
    #[must_use]
    pub fn new(dir: Option<&Path>) -> Self {
        Self {
            path: dir.map(|d| d.join("reminders.json")),
        }
    }

    /// Where sending resumes: the saved mark, but no further back than
    /// [`CATCH_UP`] before `now`.
    #[must_use]
    pub fn load(&self, now: Timestamp) -> Timestamp {
        let floor = now.saturating_sub(CATCH_UP).unwrap_or(now);
        let saved = self
            .path
            .as_ref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|bytes| serde_json::from_slice::<Mark>(&bytes).ok())
            .map(|m| m.sent_until);
        saved.map_or(now, |s| s.clamp(floor, now))
    }

    /// Saves the mark.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn save(&self, sent_until: Timestamp) -> Result<(), Error> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let fail = |e: std::io::Error| {
            Error::new(
                ErrorKind::Unavailable,
                format!("cannot save reminders.json: {e}"),
            )
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(fail)?;
        }
        let bytes = serde_json::to_vec(&Mark { sent_until })
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, bytes).map_err(fail)?;
        std::fs::rename(&temp, path).map_err(fail)
    }
}

/// A notice as the notifications service sends it.
#[must_use]
pub fn notification(notice: &Notice) -> Notification {
    Notification {
        kind: NotificationKind::Reminder,
        subject: notice.subject.clone(),
        title: match notice.kind {
            NoticeKind::Reminder => format!("Reminder: {}", notice.title),
            NoticeKind::Digest => notice.title.clone(),
        },
        body: notice.body.clone(),
        url: notice.url.clone(),
        actor: None,
        at: notice.at,
    }
}

/// The sender: the schedule plus what it needs to plan and deliver.
pub struct Sender<T> {
    tasks: Arc<TasksService>,
    dispatcher: Dispatcher<T>,
    watermark: Watermark,
    schedule: Option<Schedule>,
    planned_at: Option<Instant>,
    ticked_at: Option<Instant>,
}

impl<T: HttpTransport> Sender<T> {
    /// A sender that is not yet running.
    pub const fn new(
        tasks: Arc<TasksService>,
        dispatcher: Dispatcher<T>,
        watermark: Watermark,
    ) -> Self {
        Self {
            tasks,
            dispatcher,
            watermark,
            schedule: None,
            planned_at: None,
            ticked_at: None,
        }
    }

    /// Stops sending, as when another replica took the lease.
    pub fn stand_down(&mut self) {
        self.schedule = None;
        self.planned_at = None;
    }

    /// One tick at `now`: plans whoever needs it, then sends what is due.
    /// Returns how many notices went out.
    pub async fn tick(&mut self, now: Timestamp) -> usize {
        // A long gap means another replica held the lease meanwhile and
        // may have sent: start again from the saved watermark.
        if self.ticked_at.is_some_and(|at| at.elapsed() > 3 * TICK) {
            self.stand_down();
        }
        self.ticked_at = Some(Instant::now());
        let schedule = self
            .schedule
            .get_or_insert_with(|| Schedule::new(self.watermark.load(now)));
        let full = self.planned_at.is_none_or(|at| at.elapsed() >= PLAN_EVERY);
        let people = if full {
            let (_, _) = schedule.extend(now);
            self.planned_at = Some(Instant::now());
            let _ = self.tasks.take_edited();
            match self.tasks.people().await {
                Ok(people) => people,
                Err(error) => {
                    tracing::warn!(%error, "reminders: cannot list people");
                    Vec::new()
                }
            }
        } else {
            self.tasks.take_edited()
        };
        let (after, until) = (schedule.watermark(), schedule.horizon());
        for user in people {
            match self.tasks.notices(user, after, until).await {
                Ok(notices) => schedule.plan(user, notices),
                Err(error) => tracing::warn!(%error, %user, "reminders: planning failed"),
            }
        }
        let due = schedule.fire(now);
        for notice in &due {
            let _ = self
                .dispatcher
                .deliver(&notification(notice), None, &[notice.user], now)
                .await;
        }
        if let Err(error) = self.watermark.save(schedule.watermark()) {
            tracing::warn!(%error, "reminders: progress not saved");
        }
        due.len()
    }
}

/// Starts the sender; it runs on whichever replica holds [`LEASE`].
///
/// # Errors
///
/// The outbound HTTP client cannot be built.
pub fn start(
    replica: &Replica,
    tasks: &Arc<TasksService>,
    notify: &Arc<NotifyStore>,
    data_dir: &Path,
) -> Result<(), Error> {
    let transport = SafeTransport::new(policy(&[])).map_err(|e| {
        Error::new(
            ErrorKind::Internal,
            format!("cannot build the push client: {e}"),
        )
    })?;
    let watermark = Watermark::new(Some(&data_dir.join(".vellum").join("tasks")));
    let sender = Arc::new(Mutex::new(Sender::new(
        Arc::clone(tasks),
        Dispatcher::new(Arc::clone(notify), transport),
        watermark,
    )));
    replica.every(LEASE, TICK, move || {
        let sender = Arc::clone(&sender);
        async move {
            let sent = sender.lock().await.tick(Timestamp::now()).await;
            if sent > 0 {
                tracing::info!(sent, "reminders sent");
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_watermark_resumes_but_never_reaches_far_back() {
        let dir = tempfile::tempdir().unwrap();
        let mark = Watermark::new(Some(dir.path()));
        let now: Timestamp = "2026-11-03T10:00:00Z".parse().unwrap();
        assert_eq!(mark.load(now), now, "nothing saved starts now");
        let earlier: Timestamp = "2026-11-03T09:58:00Z".parse().unwrap();
        mark.save(earlier).unwrap();
        assert_eq!(mark.load(now), earlier);
        let days_ago: Timestamp = "2026-11-01T10:00:00Z".parse().unwrap();
        mark.save(days_ago).unwrap();
        assert_eq!(mark.load(now), now.saturating_sub(CATCH_UP).unwrap());
        let future: Timestamp = "2026-11-04T10:00:00Z".parse().unwrap();
        mark.save(future).unwrap();
        assert_eq!(mark.load(now), now, "a clock that ran ahead is pulled back");
    }

    #[test]
    fn reminders_are_urgent_notifications() {
        let notice = Notice {
            user: vellum_kernel::UserId::generate(),
            kind: NoticeKind::Reminder,
            subject: "doc.blk@1".into(),
            title: "Pay invoice".into(),
            body: "Due Tue 3 Nov".into(),
            url: "/p/doc".into(),
            at: Timestamp::UNIX_EPOCH,
        };
        let n = notification(&notice);
        assert_eq!(n.title, "Reminder: Pay invoice");
        assert!(n.kind.is_urgent());
        assert_eq!(n.actor, None);
    }
}
