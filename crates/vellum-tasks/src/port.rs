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
use vellum_kernel::{DocId, Error, ShareTokenHash, UserId};

use crate::model::Task;
use crate::projection::FileContext;

/// A signed-in person as My Work needs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// The account.
    pub user: UserId,
    /// Their handle, without `@`.
    pub handle: String,
    /// Their display name.
    pub display_name: String,
    /// Their time zone; "today" is their local date.
    pub time_zone: TimeZone,
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

    /// Issues assigned to the user, comments that assign them and review
    /// requests, already projected as tasks (R-MW-1's union).
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

    /// The Vellum user linked to a Slack user in a Slack workspace.
    fn slack_user(
        &self,
        team: String,
        slack_user: String,
    ) -> BoxFuture<'_, Result<Option<UserId>, Error>>;
}
