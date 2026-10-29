// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Comment threads on documents, blocks and issues (D129, D130).
//!
//! A thread is one YAML file, `.comments/<target id>/<thread id>.yaml`,
//! holding the root comment and its replies. Threads are filed under the
//! ID of the document or issue, not its path, so renaming a page or moving
//! an issue to another team never orphans its comments.
//!
//! [`Comments`] holds every loaded thread. Like the Work graph, each
//! command validates, applies and returns [`CommentChanges`]: the thread
//! files to write or delete, the events to publish and the people to
//! subscribe to an issue.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tracing::{info, instrument};
use vellum_kernel::{CommentId, DocId};

use crate::error::{Result, WorkError};
use crate::event::WorkEvent;
use crate::graph::Ctx;
use crate::id::{IssueId, UserRef};
use crate::mention;

/// The longest comment body accepted, in bytes.
pub const MAX_BODY: usize = 64 * 1024;

/// The most distinct reactions on one comment.
pub const MAX_REACTIONS: usize = 20;

/// A block in a document: `01JB…#blk-a1b2c3d4`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BlockRef {
    /// The document.
    pub doc: DocId,
    /// The block ID, `blk-` and eight base32 characters.
    pub block: String,
}

impl BlockRef {
    /// A reference to `block` in `doc`.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] if `block` is not a block ID.
    pub fn new(doc: DocId, block: &str) -> Result<Self> {
        let block = block.strip_prefix('^').unwrap_or(block);
        if !mention::is_block_id(block) {
            return Err(WorkError::Invalid {
                what: "block ID",
                value: block.to_owned(),
                reason: "must be blk- and eight lowercase base32 characters",
            });
        }
        Ok(Self {
            doc,
            block: block.to_owned(),
        })
    }

    /// The link stored on an issue: `vellum://doc/<id>#blk-…`.
    #[must_use]
    pub fn url(&self) -> String {
        format!("vellum://doc/{}#{}", self.doc, self.block)
    }

    /// Reads a link written by [`Self::url`].
    #[must_use]
    pub fn from_url(url: &str) -> Option<Self> {
        url.strip_prefix("vellum://doc/")?.parse().ok()
    }
}

impl fmt::Display for BlockRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.doc, self.block)
    }
}

impl FromStr for BlockRef {
    type Err = WorkError;

    fn from_str(text: &str) -> Result<Self> {
        let (doc, block) = text.split_once('#').ok_or_else(|| WorkError::Invalid {
            what: "block reference",
            value: text.to_owned(),
            reason: "must be <document id>#blk-…",
        })?;
        let doc = doc.parse::<DocId>().map_err(|_| WorkError::Invalid {
            what: "document ID",
            value: doc.to_owned(),
            reason: "must be a ULID",
        })?;
        Self::new(doc, block)
    }
}

/// What a thread is attached to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Target {
    /// An issue (its description and activity).
    Issue {
        /// The issue.
        issue: IssueId,
    },
    /// A document, or one block in it.
    Doc {
        /// The document.
        doc: DocId,
        /// The block the thread is anchored to; `None` for a page comment.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        block: Option<String>,
    },
}

impl Target {
    /// A page-level comment target.
    #[must_use]
    pub const fn doc(doc: DocId) -> Self {
        Self::Doc { doc, block: None }
    }

    /// A block comment target.
    #[must_use]
    pub fn block(at: BlockRef) -> Self {
        Self::Doc {
            doc: at.doc,
            block: Some(at.block),
        }
    }

    /// The ID of the object the thread belongs to, which names its folder.
    #[must_use]
    pub fn object(&self) -> String {
        match self {
            Self::Issue { issue } => issue.to_string(),
            Self::Doc { doc, .. } => doc.to_string(),
        }
    }

    /// The issue, when the target is one.
    #[must_use]
    pub const fn issue(&self) -> Option<IssueId> {
        match self {
            Self::Issue { issue } => Some(*issue),
            Self::Doc { .. } => None,
        }
    }

    fn check(&self) -> Result<()> {
        if let Self::Doc {
            doc,
            block: Some(block),
        } = self
        {
            let _ = BlockRef::new(*doc, block)?;
        }
        Ok(())
    }
}

/// Who wrote a comment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoredBy {
    /// The author typed it.
    #[default]
    Person,
    /// A model wrote it for the author (through MCP); shown with a badge.
    Model,
}

impl AuthoredBy {
    #[allow(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde skip_serializing_if passes a reference"
    )]
    const fn is_person(&self) -> bool {
        matches!(self, Self::Person)
    }
}

/// One comment in a thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    /// The comment ID. The first comment's ID is the thread's ID.
    pub id: CommentId,
    /// Who wrote it, or for whom a model wrote it.
    pub author: UserRef,
    /// Person or model.
    #[serde(default, skip_serializing_if = "AuthoredBy::is_person")]
    pub authored_by: AuthoredBy,
    /// When it was posted.
    #[serde(with = "crate::timestamp")]
    pub created_at: OffsetDateTime,
    /// When it was last edited.
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub edited_at: Option<OffsetDateTime>,
    /// The text, in Vellum Markdown. Empty once deleted.
    #[serde(default)]
    pub body: String,
    /// Reactions: emoji to the people who chose it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reactions: BTreeMap<String, Vec<UserRef>>,
    /// Set when the root comment was deleted but replies remain.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deleted: bool,
}

impl Comment {
    /// The people mentioned in the body.
    #[must_use]
    pub fn mentions(&self) -> Vec<UserRef> {
        mention::mentions(&self.body)
    }
}

/// A comment thread: a root comment and its replies (D129).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    /// The thread ID, equal to the root comment's ID.
    pub id: CommentId,
    /// What the thread is attached to.
    pub target: Target,
    /// The text that was selected when the thread was started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
    /// Who the thread is assigned to, from "assign to" on a comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<UserRef>,
    /// Who assigned it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assigned_by: Option<UserRef>,
    /// When it was assigned.
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub assigned_at: Option<OffsetDateTime>,
    /// When the thread was resolved; unset while open.
    #[serde(
        default,
        with = "crate::timestamp::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub resolved_at: Option<OffsetDateTime>,
    /// Who resolved it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_by: Option<UserRef>,
    /// The root comment and replies, oldest first.
    pub comments: Vec<Comment>,
    /// File format version.
    #[serde(default = "one")]
    pub vellum: u8,
}

const fn one() -> u8 {
    1
}

impl Thread {
    /// The file path relative to the workspace root.
    #[must_use]
    pub fn path(&self) -> String {
        format!(".comments/{}/{}.yaml", self.target.object(), self.id)
    }

    /// Whether the thread is resolved.
    #[must_use]
    pub const fn is_resolved(&self) -> bool {
        self.resolved_at.is_some()
    }

    /// Everyone who wrote in the thread or is assigned it.
    #[must_use]
    pub fn participants(&self) -> BTreeSet<UserRef> {
        self.comments
            .iter()
            .filter(|c| !c.deleted)
            .map(|c| c.author.clone())
            .chain(self.assignee.clone())
            .collect()
    }

    /// Comments that have not been deleted.
    pub fn live(&self) -> impl Iterator<Item = &Comment> {
        self.comments.iter().filter(|c| !c.deleted)
    }

    /// Reads a thread file.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if the YAML is invalid, the thread has
    /// no comments, or its ID does not match its first comment or path.
    pub fn from_yaml(path: &str, text: &str) -> Result<Self> {
        let file = |reason: String| WorkError::File {
            path: path.to_owned(),
            reason,
        };
        let thread: Self = serde_saphyr::from_str(text).map_err(|e| file(e.to_string()))?;
        if thread.comments.first().map(|c| c.id) != Some(thread.id) {
            return Err(file("the thread ID must be its first comment's ID".into()));
        }
        if thread.path() != path {
            return Err(file(format!(
                "a thread with this ID belongs at {}",
                thread.path()
            )));
        }
        thread.target.check()?;
        Ok(thread)
    }

    /// Writes the thread file.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if serialisation fails.
    pub fn to_yaml(&self) -> Result<String> {
        serde_saphyr::to_string(self).map_err(|e| WorkError::File {
            path: self.path(),
            reason: e.to_string(),
        })
    }
}

/// An open "assign to" from a comment, as My Work lists it (D130).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    /// The thread.
    pub thread: CommentId,
    /// Where the thread is.
    pub target: Target,
    /// The person assigned.
    pub assignee: UserRef,
    /// Who assigned it.
    pub assigned_by: Option<UserRef>,
    /// When.
    #[serde(with = "crate::timestamp::option")]
    pub assigned_at: Option<OffsetDateTime>,
    /// The quoted text, or the start of the root comment.
    pub summary: String,
}

/// The input for posting a comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewComment {
    /// The new comment's ID, from the kernel's generator.
    pub id: CommentId,
    /// Where a new thread goes; `None` for a reply.
    pub target: Option<Target>,
    /// The comment to reply to (any comment in the thread); `None` starts a
    /// thread.
    pub reply_to: Option<CommentId>,
    /// The text selected in the document, for a new block thread.
    pub quote: Option<String>,
    /// The body, in Vellum Markdown.
    pub body: String,
    /// "Assign to": gives the thread to this person.
    pub assign_to: Option<UserRef>,
    /// Person or model.
    pub authored_by: AuthoredBy,
}

impl NewComment {
    /// A new thread on `target`.
    #[must_use]
    pub fn new(id: CommentId, target: Target, body: &str) -> Self {
        Self {
            id,
            target: Some(target),
            reply_to: None,
            quote: None,
            body: body.to_owned(),
            assign_to: None,
            authored_by: AuthoredBy::Person,
        }
    }

    /// A reply to `to`.
    #[must_use]
    pub fn reply(id: CommentId, to: CommentId, body: &str) -> Self {
        Self {
            id,
            target: None,
            reply_to: Some(to),
            quote: None,
            body: body.to_owned(),
            assign_to: None,
            authored_by: AuthoredBy::Person,
        }
    }
}

/// What a comment command changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommentChanges {
    /// Threads whose files must be written.
    pub threads: BTreeSet<CommentId>,
    /// Thread files to delete.
    pub removed_paths: BTreeSet<String>,
    /// People to subscribe to the issue, for threads on an issue.
    pub subscribe: Option<(IssueId, Vec<UserRef>)>,
    /// Events, in order.
    pub events: Vec<WorkEvent>,
}

/// Every loaded comment thread.
#[derive(Debug, Clone, Default)]
pub struct Comments {
    threads: BTreeMap<CommentId, Thread>,
    by_comment: HashMap<CommentId, CommentId>,
    by_object: BTreeMap<String, BTreeSet<CommentId>>,
}

impl Comments {
    /// No threads.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a thread read from its file.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] if a comment ID in it is already
    /// loaded.
    pub fn load(&mut self, thread: Thread) -> Result<()> {
        let mut ids = BTreeSet::new();
        for c in &thread.comments {
            if self.by_comment.contains_key(&c.id) || !ids.insert(c.id) {
                return Err(WorkError::Invalid {
                    what: "comment ID",
                    value: c.id.to_string(),
                    reason: "is already used",
                });
            }
        }
        self.index(&thread);
        let _ = self.threads.insert(thread.id, thread);
        Ok(())
    }

    /// Removes the thread stored at `path` (its file changed or was
    /// removed by another replica), returning it.
    pub fn unload_path(&mut self, path: &str) -> Option<Thread> {
        let id = self.threads.values().find(|t| t.path() == path)?.id;
        let old = self.threads.remove(&id)?;
        self.unindex(&old);
        Some(old)
    }

    fn index(&mut self, thread: &Thread) {
        for c in &thread.comments {
            let _ = self.by_comment.insert(c.id, thread.id);
        }
        let _ = self
            .by_object
            .entry(thread.target.object())
            .or_default()
            .insert(thread.id);
    }

    fn unindex(&mut self, thread: &Thread) {
        for c in &thread.comments {
            let _ = self.by_comment.remove(&c.id);
        }
        if let Some(set) = self.by_object.get_mut(&thread.target.object()) {
            let _ = set.remove(&thread.id);
            if set.is_empty() {
                let _ = self.by_object.remove(&thread.target.object());
            }
        }
    }

    /// A thread by its ID.
    #[must_use]
    pub fn thread(&self, id: &CommentId) -> Option<&Thread> {
        self.threads.get(id)
    }

    /// The thread holding a comment.
    #[must_use]
    pub fn thread_of(&self, comment: &CommentId) -> Option<&Thread> {
        self.threads.get(self.by_comment.get(comment)?)
    }

    /// The threads on a document or issue, oldest first, resolved last.
    #[must_use]
    pub fn on(&self, object: &str) -> Vec<&Thread> {
        let mut out: Vec<&Thread> = self
            .by_object
            .get(object)
            .into_iter()
            .flatten()
            .filter_map(|id| self.threads.get(id))
            .collect();
        out.sort_by_key(|t| (t.is_resolved(), t.id));
        out
    }

    /// Open block threads on `doc` whose block no longer exists; the UI
    /// lists them as "original text deleted".
    #[must_use]
    pub fn orphaned(&self, doc: &DocId, live_blocks: &BTreeSet<&str>) -> Vec<&Thread> {
        self.on(&doc.to_string())
            .into_iter()
            .filter(|t| !t.is_resolved())
            .filter(|t| match &t.target {
                Target::Doc {
                    block: Some(block), ..
                } => !live_blocks.contains(block.as_str()),
                _ => false,
            })
            .collect()
    }

    /// Open "assign to" items for `user`, oldest first (My Work, D130).
    #[must_use]
    pub fn assignments(&self, user: &UserRef) -> Vec<Assignment> {
        let mut out: Vec<Assignment> = self
            .threads
            .values()
            .filter(|t| !t.is_resolved() && t.assignee.as_ref() == Some(user))
            .map(|t| Assignment {
                thread: t.id,
                target: t.target.clone(),
                assignee: user.clone(),
                assigned_by: t.assigned_by.clone(),
                assigned_at: t.assigned_at,
                summary: summary(t),
            })
            .collect();
        out.sort_by_key(|a| (a.assigned_at, a.thread));
        out
    }

    // ----- commands -----

    /// Posts a comment: a new thread, or a reply (D129).
    ///
    /// Replying to a resolved thread reopens it. `audience` is who else
    /// follows the target (the issue's subscribers, or the document's
    /// owner); they are notified along with the thread's participants and
    /// everyone mentioned.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] for an empty or oversized body, a
    /// reused ID, a bad block ID or a missing actor, and
    /// [`WorkError::UnknownIssue`] if `reply_to` is not a known comment.
    #[instrument(skip_all, fields(comment = %draft.id))]
    pub fn add(
        &mut self,
        draft: NewComment,
        audience: &[UserRef],
        ctx: &Ctx,
    ) -> Result<CommentChanges> {
        let author = actor(ctx)?;
        check_body(&draft.body)?;
        if self.by_comment.contains_key(&draft.id) {
            return Err(WorkError::Invalid {
                what: "comment ID",
                value: draft.id.to_string(),
                reason: "is already used",
            });
        }
        let comment = Comment {
            id: draft.id,
            author: author.clone(),
            authored_by: draft.authored_by,
            created_at: ctx.now,
            edited_at: None,
            body: draft.body,
            reactions: BTreeMap::new(),
            deleted: false,
        };
        let mut thread = if let Some(to) = draft.reply_to {
            let id = *self.by_comment.get(&to).ok_or_else(|| unknown(&to))?;
            self.threads.get(&id).cloned().ok_or_else(|| unknown(&id))?
        } else {
            {
                let target = draft.target.ok_or_else(|| WorkError::Invalid {
                    what: "comment",
                    value: draft.id.to_string(),
                    reason: "a new thread needs a target",
                })?;
                target.check()?;
                Thread {
                    id: draft.id,
                    target,
                    quote: draft.quote.filter(|q| !q.trim().is_empty()),
                    assignee: None,
                    assigned_by: None,
                    assigned_at: None,
                    resolved_at: None,
                    resolved_by: None,
                    comments: Vec::new(),
                    vellum: 1,
                }
            }
        };
        let mut events = Vec::new();
        if thread.is_resolved() {
            thread.resolved_at = None;
            thread.resolved_by = None;
            events.push(WorkEvent::CommentThreadReopened {
                thread: thread.id,
                target: thread.target.clone(),
                by: Some(author.clone()),
            });
        }
        let mentions = comment.mentions();
        let mut recipients: BTreeSet<UserRef> = thread.participants();
        recipients.extend(audience.iter().cloned());
        recipients.extend(mentions.iter().cloned());
        thread.comments.push(comment.clone());
        let assigned = draft.assign_to;
        if let Some(to) = &assigned {
            thread.assignee = Some(to.clone());
            thread.assigned_by = Some(author.clone());
            thread.assigned_at = Some(ctx.now);
            let _ = recipients.insert(to.clone());
        }
        let _ = recipients.remove(&author);
        events.insert(
            0,
            WorkEvent::CommentAdded {
                thread: thread.id,
                target: thread.target.clone(),
                comment: Box::new(comment),
                mentions: mentions.clone(),
                assignee: assigned,
                recipients: recipients.into_iter().collect(),
            },
        );
        let subscribe = thread.target.issue().map(|issue| {
            let mut people = vec![author];
            people.extend(mentions);
            people.extend(thread.assignee.clone());
            (issue, people)
        });
        info!(
            monotonic_counter.work_comments_added = 1_u64,
            reply = draft.reply_to.is_some()
        );
        Ok(self.store(thread, subscribe, events))
    }

    /// Edits a comment's body. Only its author may edit it; people newly
    /// mentioned are notified.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownIssue`] for an unknown comment,
    /// [`WorkError::InvalidState`] if the actor is not the author or the
    /// comment was deleted, and [`WorkError::Invalid`] for a bad body.
    #[instrument(skip_all, fields(comment = %id))]
    pub fn edit(&mut self, id: &CommentId, body: &str, ctx: &Ctx) -> Result<CommentChanges> {
        let me = actor(ctx)?;
        check_body(body)?;
        let mut thread = self.thread_of(id).cloned().ok_or_else(|| unknown(id))?;
        let comment = thread
            .comments
            .iter_mut()
            .find(|c| c.id == *id)
            .ok_or_else(|| unknown(id))?;
        if comment.author != me || comment.deleted {
            return Err(refused(id, "only the author can edit a comment"));
        }
        let before: BTreeSet<UserRef> = comment.mentions().into_iter().collect();
        body.clone_into(&mut comment.body);
        comment.edited_at = Some(ctx.now);
        let added: Vec<UserRef> = comment
            .mentions()
            .into_iter()
            .filter(|u| !before.contains(u) && *u != me)
            .collect();
        let event = WorkEvent::CommentEdited {
            thread: thread.id,
            target: thread.target.clone(),
            comment: Box::new(comment.clone()),
            new_mentions: added.clone(),
        };
        let subscribe = thread
            .target
            .issue()
            .filter(|_| !added.is_empty())
            .map(|i| (i, added));
        Ok(self.store(thread, subscribe, vec![event]))
    }

    /// Deletes a comment. Authors delete their own; `moderator` (a manager
    /// of the space, decided by the caller) may delete any.
    ///
    /// Deleting the root of a thread with live replies leaves a "deleted"
    /// marker so the replies keep their context. A thread with nothing
    /// left is removed.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownIssue`] for an unknown comment and
    /// [`WorkError::InvalidState`] if the actor may not delete it.
    #[instrument(skip_all, fields(comment = %id))]
    pub fn delete(&mut self, id: &CommentId, moderator: bool, ctx: &Ctx) -> Result<CommentChanges> {
        let me = actor(ctx)?;
        let mut thread = self.thread_of(id).cloned().ok_or_else(|| unknown(id))?;
        let index = thread
            .comments
            .iter()
            .position(|c| c.id == *id && !c.deleted)
            .ok_or_else(|| unknown(id))?;
        if !moderator && thread.comments.get(index).map(|c| &c.author) != Some(&me) {
            return Err(refused(
                id,
                "only the author or a manager can delete a comment",
            ));
        }
        let is_root = index == 0;
        if is_root {
            if let Some(root) = thread.comments.first_mut() {
                root.deleted = true;
                root.body.clear();
                root.reactions.clear();
            }
        } else {
            let _ = thread.comments.remove(index);
            let _ = self.by_comment.remove(id);
        }
        let event = WorkEvent::CommentDeleted {
            thread: thread.id,
            target: thread.target.clone(),
            comment: *id,
            by: me,
        };
        if thread.live().next().is_none() {
            let old = self
                .threads
                .remove(&thread.id)
                .unwrap_or_else(|| thread.clone());
            self.unindex(&old);
            let mut changes = CommentChanges {
                events: vec![event],
                ..CommentChanges::default()
            };
            let _ = changes.removed_paths.insert(old.path());
            return Ok(changes);
        }
        Ok(self.store(thread, None, vec![event]))
    }

    /// Adds (`on`) or removes the actor's reaction on a comment.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownIssue`] for an unknown comment and
    /// [`WorkError::Invalid`] for an empty or long emoji, or a comment that
    /// already has [`MAX_REACTIONS`] different ones.
    #[instrument(skip_all, fields(comment = %id))]
    pub fn react(
        &mut self,
        id: &CommentId,
        emoji: &str,
        on: bool,
        ctx: &Ctx,
    ) -> Result<CommentChanges> {
        let me = actor(ctx)?;
        let emoji = emoji.trim();
        if emoji.is_empty() || emoji.chars().count() > 16 || emoji.chars().any(char::is_whitespace)
        {
            return Err(WorkError::Invalid {
                what: "reaction",
                value: emoji.to_owned(),
                reason: "must be one emoji or shortcode",
            });
        }
        let mut thread = self.thread_of(id).cloned().ok_or_else(|| unknown(id))?;
        let comment = thread
            .comments
            .iter_mut()
            .find(|c| c.id == *id && !c.deleted)
            .ok_or_else(|| unknown(id))?;
        let changed = if on {
            if !comment.reactions.contains_key(emoji) && comment.reactions.len() >= MAX_REACTIONS {
                return Err(WorkError::Invalid {
                    what: "reaction",
                    value: emoji.to_owned(),
                    reason: "this comment has the most reactions allowed",
                });
            }
            let people = comment.reactions.entry(emoji.to_owned()).or_default();
            let new = !people.contains(&me);
            if new {
                people.push(me.clone());
            }
            new
        } else {
            let people = comment.reactions.get_mut(emoji);
            let had = people.as_ref().is_some_and(|p| p.contains(&me));
            if let Some(people) = people {
                people.retain(|u| *u != me);
                if people.is_empty() {
                    let _ = comment.reactions.remove(emoji);
                }
            }
            had
        };
        if !changed {
            return Ok(CommentChanges::default());
        }
        let event = WorkEvent::CommentReaction {
            thread: thread.id,
            comment: *id,
            emoji: emoji.to_owned(),
            by: me,
            added: on,
        };
        Ok(self.store(thread, None, vec![event]))
    }

    /// Resolves or reopens a thread. Resolving completes its assignment.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownIssue`] for an unknown thread.
    #[instrument(skip_all, fields(thread = %id))]
    pub fn resolve(&mut self, id: &CommentId, resolved: bool, ctx: &Ctx) -> Result<CommentChanges> {
        let mut thread = self.thread_of(id).cloned().ok_or_else(|| unknown(id))?;
        if thread.is_resolved() == resolved {
            return Ok(CommentChanges::default());
        }
        let event = if resolved {
            thread.resolved_at = Some(ctx.now);
            thread.resolved_by.clone_from(&ctx.actor);
            WorkEvent::CommentThreadResolved {
                thread: thread.id,
                target: thread.target.clone(),
                by: ctx.actor.clone(),
                assignee: thread.assignee.clone(),
            }
        } else {
            thread.resolved_at = None;
            thread.resolved_by = None;
            WorkEvent::CommentThreadReopened {
                thread: thread.id,
                target: thread.target.clone(),
                by: ctx.actor.clone(),
            }
        };
        Ok(self.store(thread, None, vec![event]))
    }

    /// Reassigns or unassigns a thread without posting a comment.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownIssue`] for an unknown thread.
    #[instrument(skip_all, fields(thread = %id))]
    pub fn assign(
        &mut self,
        id: &CommentId,
        to: Option<UserRef>,
        ctx: &Ctx,
    ) -> Result<CommentChanges> {
        let mut thread = self.thread_of(id).cloned().ok_or_else(|| unknown(id))?;
        if thread.assignee == to {
            return Ok(CommentChanges::default());
        }
        let from = thread.assignee.take();
        thread.assignee.clone_from(&to);
        thread.assigned_by = to.as_ref().and_then(|_| ctx.actor.clone());
        thread.assigned_at = to.as_ref().map(|_| ctx.now);
        let event = WorkEvent::CommentAssigned {
            thread: thread.id,
            target: thread.target.clone(),
            from,
            to: to.clone(),
            by: ctx.actor.clone(),
        };
        let subscribe = thread
            .target
            .issue()
            .zip(to)
            .map(|(issue, user)| (issue, vec![user]));
        Ok(self.store(thread, subscribe, vec![event]))
    }

    /// How many threads a document or issue has, resolved ones included.
    #[must_use]
    pub fn threads_on(&self, object: &str) -> usize {
        self.by_object.get(object).map_or(0, BTreeSet::len)
    }

    fn store(
        &mut self,
        thread: Thread,
        subscribe: Option<(IssueId, Vec<UserRef>)>,
        events: Vec<WorkEvent>,
    ) -> CommentChanges {
        if let Some(old) = self.threads.remove(&thread.id) {
            self.unindex(&old);
        }
        self.index(&thread);
        let mut changes = CommentChanges {
            subscribe: subscribe.map(|(i, mut people)| {
                people.sort();
                people.dedup();
                (i, people)
            }),
            events,
            ..CommentChanges::default()
        };
        let _ = changes.threads.insert(thread.id);
        let _ = self.threads.insert(thread.id, thread);
        changes
    }
}

fn actor(ctx: &Ctx) -> Result<UserRef> {
    ctx.actor.clone().ok_or_else(|| WorkError::Invalid {
        what: "actor",
        value: String::new(),
        reason: "comments need a person",
    })
}

fn check_body(body: &str) -> Result<()> {
    if body.trim().is_empty() {
        return Err(WorkError::Invalid {
            what: "comment",
            value: String::new(),
            reason: "must not be empty",
        });
    }
    if body.len() > MAX_BODY {
        return Err(WorkError::Invalid {
            what: "comment",
            value: format!("{} bytes", body.len()),
            reason: "must be at most 64 KiB",
        });
    }
    Ok(())
}

fn unknown(id: &CommentId) -> WorkError {
    WorkError::UnknownIssue(format!("comment {id}"))
}

fn refused(id: &CommentId, reason: &'static str) -> WorkError {
    WorkError::InvalidState {
        issue: format!("comment {id}"),
        reason,
    }
}

fn summary(thread: &Thread) -> String {
    let text = thread
        .quote
        .as_deref()
        .or_else(|| thread.live().next().map(|c| c.body.as_str()))
        .unwrap_or_default();
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() > 120 {
        let cut: String = line.chars().take(119).collect();
        format!("{cut}…")
    } else {
        line.to_owned()
    }
}

impl crate::graph::WorkGraph {
    /// Reads a comment target as the API and MCP write it: an issue key
    /// (`ENG-142`, aliases too), an issue ID, `<doc id>#blk-…` or a
    /// document ID.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownIssue`] for a key of no issue and
    /// [`WorkError::Invalid`] for anything else unreadable.
    pub fn comment_target(&self, text: &str) -> Result<Target> {
        let text = text.trim();
        if text.contains('#') {
            return text.parse::<BlockRef>().map(Target::block);
        }
        if crate::issue::IssueKey::parse(text).is_ok() {
            return self
                .resolve(text)
                .map(|i| Target::Issue { issue: i.id })
                .ok_or_else(|| WorkError::UnknownIssue(text.to_owned()));
        }
        if let Ok(id) = text.parse::<IssueId>()
            && self.issue(&id).is_some()
        {
            return Ok(Target::Issue { issue: id });
        }
        text.parse::<DocId>()
            .map(Target::doc)
            .map_err(|_| WorkError::Invalid {
                what: "comment target",
                value: text.to_owned(),
                reason: "must be an issue key, a document ID or <document id>#blk-…",
            })
    }

    /// Applies the issue side of a comment command: commenters, people
    /// mentioned and assignees subscribe to the issue (D130).
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::UnknownIssue`] if the issue is gone.
    pub fn apply_comment(
        &mut self,
        changes: &CommentChanges,
        ctx: &Ctx,
    ) -> Result<crate::graph::Changes> {
        let Some((issue, people)) = &changes.subscribe else {
            return Ok(crate::graph::Changes::default());
        };
        let current = &self
            .issue(issue)
            .ok_or_else(|| WorkError::UnknownIssue(issue.to_string()))?
            .subscribers;
        let new: Vec<UserRef> = people
            .iter()
            .filter(|p| !current.contains(p))
            .cloned()
            .collect();
        if new.is_empty() {
            return Ok(crate::graph::Changes::default());
        }
        let patch = crate::graph::IssuePatch {
            subscribe: new,
            ..crate::graph::IssuePatch::default()
        };
        self.update_issue(issue, patch, ctx)
    }
}
