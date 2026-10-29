// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Notifications to people: the in-app inbox, email and web push.
//!
//! A [`Notification`] is one thing a person should hear about. [`plan`]
//! turns it into channel deliveries using each recipient's
//! [`Preferences`]:
//!
//! - the **inbox** always gets it unless the kind is switched off;
//! - **web push** goes out at once to every registered browser;
//! - **email** waits [`Preferences::email_delay`] and is skipped if the
//!   person has read the inbox entry by then, so an active user is not
//!   emailed about what they already saw (D80).
//!
//! The caller passes only recipients who can read the subject; this
//! module never widens access.

use std::collections::{BTreeMap, BTreeSet};

use jiff::civil::Time;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

/// A delivery channel to a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// The in-app inbox.
    InApp,
    /// Email.
    Email,
    /// Web push to the PWA and desktop app.
    Push,
}

/// What happened, from the recipient's point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    /// An issue was assigned to you.
    IssueAssigned,
    /// You were mentioned in an issue, comment or document.
    Mentioned,
    /// Someone commented on something you subscribe to.
    Commented,
    /// An issue you subscribe to changed state.
    StateChanged,
    /// A new issue arrived in a triage inbox you own.
    TriageNew,
    /// An SLA on your issue is at high risk.
    SlaHighRisk,
    /// An SLA on your issue breached.
    SlaBreached,
    /// Your review was requested (a pull request or a document).
    ReviewRequested,
    /// A project you follow posted a health update.
    ProjectUpdate,
    /// A task reminder fired.
    Reminder,
    /// A Git integration linked or merged a pull request on your issue.
    PullRequest,
}

impl NotificationKind {
    /// Every kind, in a stable order.
    pub const ALL: [Self; 11] = [
        Self::IssueAssigned,
        Self::Mentioned,
        Self::Commented,
        Self::StateChanged,
        Self::TriageNew,
        Self::SlaHighRisk,
        Self::SlaBreached,
        Self::ReviewRequested,
        Self::ProjectUpdate,
        Self::Reminder,
        Self::PullRequest,
    ];

    /// The channels on by default for a new member.
    #[must_use]
    pub fn default_channels(self) -> BTreeSet<Channel> {
        let channels: &[Channel] = match self {
            Self::IssueAssigned
            | Self::Mentioned
            | Self::SlaBreached
            | Self::ReviewRequested
            | Self::Reminder => &[Channel::InApp, Channel::Email, Channel::Push],
            Self::SlaHighRisk | Self::TriageNew => &[Channel::InApp, Channel::Push],
            Self::Commented | Self::ProjectUpdate => &[Channel::InApp, Channel::Email],
            Self::StateChanged | Self::PullRequest => &[Channel::InApp],
        };
        channels.iter().copied().collect()
    }

    /// Urgent kinds bypass the email delay and quiet hours.
    #[must_use]
    pub const fn is_urgent(self) -> bool {
        matches!(self, Self::SlaBreached | Self::Reminder)
    }
}

/// One person's notification settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    /// Channels per kind; a missing kind uses its defaults.
    pub channels: BTreeMap<NotificationKind, BTreeSet<Channel>>,
    /// How long email waits for the inbox entry to be read.
    pub email_delay: SignedDuration,
    /// Subjects (issue keys, document IDs) the person muted.
    pub muted: BTreeSet<String>,
    /// Non-urgent push and email are held until this time.
    pub snoozed_until: Option<Timestamp>,
    /// The local time the daily digest email goes out; `None` turns it
    /// off (R-MW-5).
    #[serde(default = "default_digest_at")]
    pub digest_at: Option<Time>,
}

/// The digest goes out at 07:30 local time unless the person changes it.
pub const DEFAULT_DIGEST_AT: Time = Time::constant(7, 30, 0, 0);

#[expect(
    clippy::unnecessary_wraps,
    reason = "serde's default must return the field's type"
)]
const fn default_digest_at() -> Option<Time> {
    Some(DEFAULT_DIGEST_AT)
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            channels: BTreeMap::new(),
            email_delay: SignedDuration::from_mins(10),
            muted: BTreeSet::new(),
            snoozed_until: None,
            digest_at: default_digest_at(),
        }
    }
}

impl Preferences {
    /// The channels on for `kind`.
    #[must_use]
    pub fn channels_for(&self, kind: NotificationKind) -> BTreeSet<Channel> {
        self.channels
            .get(&kind)
            .cloned()
            .unwrap_or_else(|| kind.default_channels())
    }
}

/// Something one or more people should hear about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// The kind.
    pub kind: NotificationKind,
    /// What it is about: an issue key, document ID or task ID.
    pub subject: String,
    /// One line, already localised for the recipient's language.
    pub title: String,
    /// Short body text.
    pub body: String,
    /// Link into the app.
    pub url: String,
    /// The person who caused it; never notified about their own action.
    pub actor: Option<String>,
    /// When it happened.
    pub at: Timestamp,
}

impl Notification {
    /// Inbox entries with the same key collapse into one, newest first, so a
    /// burst of edits to one issue is one line.
    #[must_use]
    pub fn group_key(&self, recipient: &str) -> String {
        format!("{recipient}:{}", self.subject)
    }
}

/// A recipient and their settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipient {
    /// The user ID.
    pub user: String,
    /// Their settings.
    pub preferences: Preferences,
}

/// One delivery the job queue should run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    /// Who receives it.
    pub user: String,
    /// The channel.
    pub channel: Channel,
    /// Earliest send time.
    pub not_before: Timestamp,
    /// Email only: skip if the inbox entry is read by `not_before`.
    pub skip_if_read: bool,
}

/// Plans the deliveries of `notification` to `recipients`.
#[must_use]
pub fn plan(notification: &Notification, recipients: &[Recipient]) -> Vec<Planned> {
    let mut seen = BTreeSet::new();
    let mut planned = Vec::new();
    for recipient in recipients {
        if notification.actor.as_deref() == Some(recipient.user.as_str())
            || !seen.insert(recipient.user.as_str())
        {
            continue;
        }
        let prefs = &recipient.preferences;
        let urgent = notification.kind.is_urgent();
        if prefs.muted.contains(&notification.subject) && !urgent {
            continue;
        }
        let held_until = prefs
            .snoozed_until
            .filter(|until| !urgent && *until > notification.at)
            .unwrap_or(notification.at);
        for channel in prefs.channels_for(notification.kind) {
            let (not_before, skip_if_read) = match channel {
                Channel::InApp => (notification.at, false),
                Channel::Push => (held_until, false),
                Channel::Email if urgent => (notification.at, false),
                Channel::Email => {
                    let delayed = notification
                        .at
                        .checked_add(prefs.email_delay)
                        .unwrap_or(notification.at);
                    (delayed.max(held_until), true)
                }
            };
            planned.push(Planned {
                user: recipient.user.clone(),
                channel,
                not_before,
                skip_if_read,
            });
        }
    }
    planned
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(minutes: i64) -> Timestamp {
        Timestamp::from_second(minutes * 60).unwrap_or(Timestamp::UNIX_EPOCH)
    }

    fn note(kind: NotificationKind) -> Notification {
        Notification {
            kind,
            subject: "ENG-142".to_owned(),
            title: "Ada assigned ENG-142 to you".to_owned(),
            body: "Fix login redirect".to_owned(),
            url: "https://vellum.northstar.dev/issue/ENG-142".to_owned(),
            actor: Some("ada".to_owned()),
            at: at(0),
        }
    }

    fn person(user: &str) -> Recipient {
        Recipient {
            user: user.to_owned(),
            preferences: Preferences::default(),
        }
    }

    #[test]
    fn assignment_goes_everywhere_with_delayed_email() {
        let planned = plan(&note(NotificationKind::IssueAssigned), &[person("bo")]);
        let by_channel: BTreeMap<Channel, &Planned> =
            planned.iter().map(|p| (p.channel, p)).collect();
        assert_eq!(by_channel.len(), 3);
        assert_eq!(
            by_channel.get(&Channel::InApp).map(|p| p.not_before),
            Some(at(0))
        );
        assert_eq!(
            by_channel.get(&Channel::Push).map(|p| p.not_before),
            Some(at(0))
        );
        let email = by_channel.get(&Channel::Email).copied();
        assert_eq!(
            email.map(|p| (p.not_before, p.skip_if_read)),
            Some((at(10), true))
        );
    }

    #[test]
    fn actor_and_duplicates_are_skipped() {
        let planned = plan(
            &note(NotificationKind::StateChanged),
            &[person("ada"), person("bo"), person("bo")],
        );
        assert_eq!(planned.len(), 1);
        assert_eq!(planned.first().map(|p| p.user.as_str()), Some("bo"));
    }

    #[test]
    fn muted_and_snoozed() {
        let mut muted = person("bo");
        let _ = muted.preferences.muted.insert("ENG-142".to_owned());
        assert!(
            plan(
                &note(NotificationKind::Commented),
                std::slice::from_ref(&muted)
            )
            .is_empty()
        );
        // Urgent kinds pierce a mute.
        assert!(!plan(&note(NotificationKind::SlaBreached), &[muted]).is_empty());

        let mut snoozed = person("cy");
        snoozed.preferences.snoozed_until = Some(at(60));
        let planned = plan(&note(NotificationKind::IssueAssigned), &[snoozed]);
        for p in &planned {
            let expected = if p.channel == Channel::InApp {
                at(0)
            } else {
                at(60)
            };
            assert_eq!(p.not_before, expected, "{:?}", p.channel);
        }
    }

    #[test]
    fn custom_channels_override_defaults() {
        let mut quiet = person("bo");
        let _ = quiet
            .preferences
            .channels
            .insert(NotificationKind::IssueAssigned, BTreeSet::new());
        assert!(plan(&note(NotificationKind::IssueAssigned), &[quiet]).is_empty());
        for kind in NotificationKind::ALL {
            assert!(
                kind.default_channels().contains(&Channel::InApp),
                "{kind:?}"
            );
        }
    }
}
