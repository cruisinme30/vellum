// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! What My Work needs from the rest of the server.
//!
//! The task service owns no storage. It reads and writes Markdown files,
//! people and secrets through [`Workspace`], which the binary implements
//! over the store, the index, identity and authorization. Every call is
//! made on behalf of a user, and the implementation enforces that user's
//! access: the service never sees a file the user could not open.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use vellum_contracts::BoxFuture;
use vellum_kernel::{CommentId, DocId, Error, ShareTokenHash, UserId};

use crate::model::Task;
use crate::projection::FileContext;
use crate::settings::Preferences;

/// A signed-in person as My Work needs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// The account.
    pub user: UserId,
    /// Their handle, without `@`.
    pub handle: String,
    /// Their display name.
    pub display_name: String,
    /// The zone My Work counts in for them: their own choice, else the
    /// server's. "Today" is their local date.
    pub time_zone: TimeZone,
    /// Their My Work settings.
    pub preferences: Preferences,
}

/// A Markdown file that may hold checklist tasks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Which file it is, as the index knows it.
    pub context: FileContext,
    /// Its text.
    pub text: String,
    /// Its version; a write must name the version it was based on.
    pub version: u64,
    /// When it was last saved.
    pub modified: Timestamp,
}

/// A per-user secret that stands in for a sign-in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SecretKind {
    /// The token in the calendar feed URL.
    Feed,
    /// The token in the capture e-mail address.
    Capture,
}

/// The server, as My Work sees it.
pub trait Workspace: Send + Sync + 'static {
    /// The user's profile.
    fn profile(&self, user: UserId) -> BoxFuture<'_, Result<Profile, Error>>;

    /// The files whose checklists can hold the user's tasks: their Inbox
    /// and lists, lists shared with them, and documents that mention them.
    fn task_files(&self, user: UserId) -> BoxFuture<'_, Result<Vec<Document>, Error>>;

    /// Issues assigned to the user, comments that assign them, comments
    /// that mention them and that they have not answered or dismissed, and
    /// review requests, already projected as tasks (R-MW-1's union).
    fn assigned(&self, user: UserId) -> BoxFuture<'_, Result<Vec<Task>, Error>>;

    /// Reads a file the user can open.
    fn read(&self, user: UserId, doc: DocId) -> BoxFuture<'_, Result<Document, Error>>;

    /// Replaces a file's text if it is still at `version`, returning the
    /// new version. A newer version fails with `conflict`.
    fn write(
        &self,
        user: UserId,
        doc: DocId,
        text: String,
        version: u64,
    ) -> BoxFuture<'_, Result<u64, Error>>;

    /// Creates a list file. `path` is the suggested location relative to
    /// the user's own space (`tasks/<handle>/<slug>.md`), and the text
    /// carries a proposed `id` in its front matter. Returns the ID the
    /// stored document actually has, which an implementation may assign
    /// itself (rewriting the front matter to match).
    fn create(
        &self,
        user: UserId,
        path: String,
        text: String,
    ) -> BoxFuture<'_, Result<DocId, Error>>;

    /// Grants (`member` true) or revokes another person's edit access to
    /// a list. Unknown handles fail with `not_found`.
    fn set_member(
        &self,
        user: UserId,
        list: DocId,
        handle: String,
        member: bool,
    ) -> BoxFuture<'_, Result<(), Error>>;

    /// Stores the hash of a user's secret, replacing the old one; `None`
    /// revokes it.
    fn set_secret(
        &self,
        user: UserId,
        kind: SecretKind,
        hash: Option<ShareTokenHash>,
    ) -> BoxFuture<'_, Result<(), Error>>;

    /// The user a secret belongs to, if any. Implementations compare
    /// hashes in constant time.
    fn user_for_secret(
        &self,
        kind: SecretKind,
        hash: ShareTokenHash,
    ) -> BoxFuture<'_, Result<Option<UserId>, Error>>;

    /// Stores the user's My Work settings.
    fn save_preferences(
        &self,
        user: UserId,
        preferences: Preferences,
    ) -> BoxFuture<'_, Result<(), Error>>;

    /// Takes a mention out of the user's My Work without answering it.
    fn dismiss_mention(&self, user: UserId, comment: CommentId)
    -> BoxFuture<'_, Result<(), Error>>;

    /// Everyone whose reminders and digest the server sends.
    fn people(&self) -> BoxFuture<'_, Result<Vec<UserId>, Error>>;

    /// The Vellum user linked to a Slack user in a Slack workspace.
    fn slack_user(
        &self,
        team: String,
        slack_user: String,
    ) -> BoxFuture<'_, Result<Option<UserId>, Error>>;

    /// Links a Slack user to `user`, replacing any earlier link of that
    /// Slack user (#163). The caller has checked the Slack user's code.
    fn link_slack(
        &self,
        user: UserId,
        team: String,
        slack_user: String,
    ) -> BoxFuture<'_, Result<(), Error>> {
        let _ = (user, team, slack_user);
        Box::pin(async {
            Err(Error::new(
                vellum_kernel::ErrorKind::Unavailable,
                "Slack linking is not available here",
            ))
        })
    }

    /// Removes every Slack link of `user`; returns how many there were.
    fn unlink_slack(&self, user: UserId) -> BoxFuture<'_, Result<usize, Error>> {
        let _ = user;
        Box::pin(async { Ok(0) })
    }

    /// Posts a reply only the Slack user sees to a Slack `response_url`
    /// (always under `https://hooks.slack.com/`). Best effort.
    fn slack_say(&self, response_url: String, text: String) -> BoxFuture<'_, ()> {
        let _ = (response_url, text);
        Box::pin(async {})
    }
}
