// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Domain events: facts about what changed, published after a write commits.
//!
//! The index, search, render cache, notifications, webhooks and audit log all
//! consume these. Events carry IDs, not content; consumers re-read what they
//! need. Both enums are `#[non_exhaustive]`, so adding a kind or a field is
//! not a breaking change.

use serde::{Deserialize, Serialize};

use crate::id::{DocId, EventId, RequestId, SpaceId, UserId, VersionId};
use crate::timestamp::Timestamp;

/// Who caused a change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Actor {
    /// A signed-in person.
    User {
        /// The person.
        user: UserId,
    },
    /// A model acting through MCP for a person; its writes are drafts.
    Model {
        /// The person the model acts for.
        on_behalf_of: UserId,
    },
    /// An edit made outside Vellum and picked up by the file watcher.
    External,
    /// Vellum itself (rollover, retention, reindex).
    System,
}

/// What happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventKind {
    /// A document was created.
    #[non_exhaustive]
    DocCreated {
        /// The document.
        doc: DocId,
    },
    /// A new version of a document's content was written.
    #[non_exhaustive]
    DocUpdated {
        /// The document.
        doc: DocId,
        /// The version written.
        version: VersionId,
    },
    /// A document's title, and so its file name, changed.
    #[non_exhaustive]
    DocRenamed {
        /// The document.
        doc: DocId,
    },
    /// A document moved to another folder, drive or space.
    #[non_exhaustive]
    DocMoved {
        /// The document.
        doc: DocId,
        /// The space it left; equals the event's space for in-space moves.
        from_space: SpaceId,
    },
    /// A document was moved to the trash.
    #[non_exhaustive]
    DocTrashed {
        /// The document.
        doc: DocId,
    },
    /// A document was restored from the trash.
    #[non_exhaustive]
    DocRestored {
        /// The document.
        doc: DocId,
    },
    /// A document was permanently deleted.
    #[non_exhaustive]
    DocPurged {
        /// The document.
        doc: DocId,
    },
    /// A three-way merge could not reconcile edits and wrote a conflict copy.
    #[non_exhaustive]
    ConflictCopyCreated {
        /// The document that was being merged.
        doc: DocId,
        /// The new document holding the conflicting side.
        copy: DocId,
    },
}

impl EventKind {
    /// Builds [`EventKind::DocCreated`].
    #[must_use]
    pub const fn doc_created(doc: DocId) -> Self {
        Self::DocCreated { doc }
    }

    /// Builds [`EventKind::DocUpdated`].
    #[must_use]
    pub const fn doc_updated(doc: DocId, version: VersionId) -> Self {
        Self::DocUpdated { doc, version }
    }

    /// Builds [`EventKind::DocRenamed`].
    #[must_use]
    pub const fn doc_renamed(doc: DocId) -> Self {
        Self::DocRenamed { doc }
    }

    /// Builds [`EventKind::DocMoved`].
    #[must_use]
    pub const fn doc_moved(doc: DocId, from_space: SpaceId) -> Self {
        Self::DocMoved { doc, from_space }
    }

    /// Builds [`EventKind::DocTrashed`].
    #[must_use]
    pub const fn doc_trashed(doc: DocId) -> Self {
        Self::DocTrashed { doc }
    }

    /// Builds [`EventKind::DocRestored`].
    #[must_use]
    pub const fn doc_restored(doc: DocId) -> Self {
        Self::DocRestored { doc }
    }

    /// Builds [`EventKind::DocPurged`].
    #[must_use]
    pub const fn doc_purged(doc: DocId) -> Self {
        Self::DocPurged { doc }
    }

    /// Builds [`EventKind::ConflictCopyCreated`].
    #[must_use]
    pub const fn conflict_copy_created(doc: DocId, copy: DocId) -> Self {
        Self::ConflictCopyCreated { doc, copy }
    }
}

/// One domain event with its envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Event {
    /// Unique and time-ordered; consumers use it to de-duplicate.
    pub id: EventId,
    /// When the change committed.
    pub at: Timestamp,
    /// The shard key: the space the change happened in.
    pub space: SpaceId,
    /// Who caused it.
    pub actor: Actor,
    /// The request that caused it, if any, for trace correlation.
    pub request: Option<RequestId>,
    /// What happened.
    pub kind: EventKind,
}

impl Event {
    /// Creates an event stamped with a fresh ID and the current time.
    #[must_use]
    pub fn new(space: SpaceId, actor: Actor, kind: EventKind) -> Self {
        Self {
            id: EventId::generate(),
            at: Timestamp::now(),
            space,
            actor,
            request: None,
            kind,
        }
    }

    /// Records the request that caused the event.
    #[must_use]
    pub const fn with_request(mut self, request: RequestId) -> Self {
        self.request = Some(request);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialises_with_type_tags() -> Result<(), serde_json::Error> {
        let doc: DocId = DocId::from_u128(1);
        let event = Event::new(
            SpaceId::from_u128(2),
            Actor::Model {
                on_behalf_of: UserId::from_u128(3),
            },
            EventKind::doc_created(doc),
        )
        .with_request(RequestId::from_u128(4));
        let json = serde_json::to_value(&event)?;
        assert_eq!(
            json.pointer("/kind/type").and_then(|v| v.as_str()),
            Some("doc_created")
        );
        assert_eq!(
            json.pointer("/kind/doc").and_then(|v| v.as_str()),
            Some("00000000000000000000000001")
        );
        assert_eq!(
            json.pointer("/actor/type").and_then(|v| v.as_str()),
            Some("model")
        );
        assert_eq!(serde_json::from_value::<Event>(json)?, event);
        Ok(())
    }
}
