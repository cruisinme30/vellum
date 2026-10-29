// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Relations, permissions, subjects, objects and relation tuples.
//!
//! The vocabulary follows Zanzibar: a [`Tuple`] says
//! `object#relation@subject`, and a group subject is the userset
//! `group:<id>#member`.

use std::fmt;

use vellum_kernel::id::{
    BookId, CollectionId, DocId, DriveId, FolderId, ObjectId, ShareId, SpaceId, UserId,
};

/// A 128-bit object identity, stored as a 16-byte blob in SQLite and a
/// `uuid` in Postgres.
///
/// It carries the same bits as the `vellum-kernel` IDs and converts to and from each of them, because tuples mix kinds (a
/// space, a folder, a document, a user, a share link, a group).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Id(pub u128);

impl Id {
    /// The all-zero ID: the workspace directory, where group membership
    /// tuples live because groups span spaces (D53).
    pub const DIRECTORY: Self = Self(0);

    /// The big-endian bytes, as the index stores them.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0.to_be_bytes()
    }

    /// Reads big-endian bytes from the index.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(u128::from_be_bytes(bytes))
    }
}

impl fmt::Display for Id {
    /// The ULID text form, as kernel IDs print.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&ObjectId::from_u128(self.0), f)
    }
}

macro_rules! kernel_ids {
    ($($kind:ident),*) => {$(
        impl From<$kind> for Id {
            fn from(id: $kind) -> Self {
                Self(id.as_u128())
            }
        }

        impl From<Id> for $kind {
            fn from(id: Id) -> Self {
                Self::from_u128(id.0)
            }
        }
    )*};
}

kernel_ids!(
    ObjectId,
    SpaceId,
    DocId,
    BookId,
    CollectionId,
    DriveId,
    FolderId,
    UserId,
    ShareId
);

/// A role on an object. Each role implies every role below it, so the
/// derived ordering is the implication order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Relation {
    /// Read the object.
    Viewer,
    /// Read and comment.
    Commenter,
    /// Read, comment and edit content.
    Editor,
    /// Everything an editor can do, plus share, move, trash and purge.
    Manager,
    /// Everything a manager can do, plus transfer ownership.
    Owner,
}

impl Relation {
    /// Every relation, weakest first.
    pub const ALL: [Self; 5] = [
        Self::Viewer,
        Self::Commenter,
        Self::Editor,
        Self::Manager,
        Self::Owner,
    ];

    /// The name stored in `grants.relation` and `effective_access.relation`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Commenter => "commenter",
            Self::Editor => "editor",
            Self::Manager => "manager",
            Self::Owner => "owner",
        }
    }

    /// Parses a stored relation name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.as_str() == name)
    }

    /// Whether holding `self` satisfies a requirement of `required`.
    #[must_use]
    pub fn implies(self, required: Self) -> bool {
        self >= required
    }
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An action a caller wants to take. Each maps to the weakest relation
/// that allows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    /// Open, search, export or read through MCP.
    Read,
    /// Add or resolve comments.
    Comment,
    /// Change content.
    Edit,
    /// Change who has access.
    Share,
    /// Move, trash, restore or purge.
    Manage,
    /// Hand ownership to someone else.
    TransferOwnership,
}

impl Permission {
    /// The weakest relation that grants this permission.
    #[must_use]
    pub const fn required(self) -> Relation {
        match self {
            Self::Read => Relation::Viewer,
            Self::Comment => Relation::Commenter,
            Self::Edit => Relation::Editor,
            Self::Share | Self::Manage => Relation::Manager,
            Self::TransferOwnership => Relation::Owner,
        }
    }

    /// The name used in spans and audit events.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Comment => "comment",
            Self::Edit => "edit",
            Self::Share => "share",
            Self::Manage => "manage",
            Self::TransferOwnership => "transfer_ownership",
        }
    }
}

/// Who a tuple grants to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subject {
    /// One person.
    User(Id),
    /// Every member of a group, transitively (`group:<id>#member`).
    Group(Id),
    /// Whoever presents a share link; the ID is the link's, never the token.
    ShareLink(Id),
}

impl Subject {
    /// `(subject_type, subject_id, subject_relation)` as stored in `grants`.
    #[must_use]
    pub const fn columns(self) -> (&'static str, Id, &'static str) {
        match self {
            Self::User(id) => ("user", id, ""),
            Self::Group(id) => ("group", id, "member"),
            Self::ShareLink(id) => ("share_link", id, ""),
        }
    }

    /// Reads the stored columns back.
    #[must_use]
    pub fn from_columns(kind: &str, id: Id, relation: &str) -> Option<Self> {
        match (kind, relation) {
            ("user", "") => Some(Self::User(id)),
            ("group", "member") => Some(Self::Group(id)),
            ("share_link", "") => Some(Self::ShareLink(id)),
            _ => None,
        }
    }

    /// A principal is a subject that can present itself at check time;
    /// groups are expanded away before materialisation.
    #[must_use]
    pub const fn principal(self) -> Option<Principal> {
        match self {
            Self::User(id) => Some(Principal::User(id)),
            Self::ShareLink(id) => Some(Principal::ShareLink(id)),
            Self::Group(_) => None,
        }
    }
}

/// Whoever is asking: the subject of a check and of an
/// `effective_access` row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Principal {
    /// A signed-in person (or an MCP client acting for them).
    User(Id),
    /// An anonymous holder of a share link.
    ShareLink(Id),
}

impl Principal {
    /// The ID stored in `effective_access.subject_id`. User and link IDs
    /// are both ULIDs, so they never collide.
    #[must_use]
    pub const fn id(self) -> Id {
        match self {
            Self::User(id) | Self::ShareLink(id) => id,
        }
    }
}

impl From<UserId> for Principal {
    fn from(id: UserId) -> Self {
        Self::User(id.into())
    }
}

impl From<ShareId> for Principal {
    fn from(id: ShareId) -> Self {
        Self::ShareLink(id.into())
    }
}

impl From<Principal> for Subject {
    fn from(p: Principal) -> Self {
        match p {
            Principal::User(id) => Self::User(id),
            Principal::ShareLink(id) => Self::ShareLink(id),
        }
    }
}

/// What a tuple is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Object {
    /// A whole space (shared drive, My Drive, team, task list). Grants
    /// here reach every node in it.
    Space(Id),
    /// A folder, document, book or book section: anything in
    /// `page_ancestors`.
    Node(Id),
    /// A group; its only relation is `member`.
    Group(Id),
}

impl Object {
    /// `(object_type, object_id)` as stored in `grants`.
    #[must_use]
    pub const fn columns(self) -> (&'static str, Id) {
        match self {
            Self::Space(id) => ("space", id),
            Self::Node(id) => ("node", id),
            Self::Group(id) => ("group", id),
        }
    }

    /// Reads the stored columns back.
    #[must_use]
    pub fn from_columns(kind: &str, id: Id) -> Option<Self> {
        match kind {
            "space" => Some(Self::Space(id)),
            "node" => Some(Self::Node(id)),
            "group" => Some(Self::Group(id)),
            _ => None,
        }
    }
}

/// The relation half of a tuple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TupleRelation {
    /// A role on a space or node.
    Role(Relation),
    /// `group#member@subject`.
    Member,
    /// `node#restricted@node`: the node narrows inherited access (book
    /// sections, R-BK-9). The subject is the node itself.
    Restricted,
}

impl TupleRelation {
    /// The name stored in `grants.relation`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Role(r) => r.as_str(),
            Self::Member => "member",
            Self::Restricted => "restricted",
        }
    }

    /// Parses a stored relation name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "member" => Some(Self::Member),
            "restricted" => Some(Self::Restricted),
            other => Relation::parse(other).map(Self::Role),
        }
    }
}

/// One row of `grants`: `object#relation@subject`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tuple {
    /// What the tuple is about.
    pub object: Object,
    /// The relation it establishes.
    pub relation: TupleRelation,
    /// Who it is for. For [`TupleRelation::Restricted`] this is
    /// `Subject::User(node)` by convention and is ignored.
    pub subject: Subject,
    /// When the tuple stops counting, in UTC epoch milliseconds.
    pub expires_at: Option<i64>,
}

impl Tuple {
    /// A role grant.
    #[must_use]
    pub const fn role(object: Object, relation: Relation, subject: Subject) -> Self {
        Self {
            object,
            relation: TupleRelation::Role(relation),
            subject,
            expires_at: None,
        }
    }

    /// A group membership.
    #[must_use]
    pub const fn member(group: Id, subject: Subject) -> Self {
        Self {
            object: Object::Group(group),
            relation: TupleRelation::Member,
            subject,
            expires_at: None,
        }
    }

    /// A narrowing marker on a node.
    #[must_use]
    pub const fn restricted(node: Id) -> Self {
        Self {
            object: Object::Node(node),
            relation: TupleRelation::Restricted,
            subject: Subject::User(node),
            expires_at: None,
        }
    }

    /// The same tuple with an expiry.
    #[must_use]
    pub const fn expiring(mut self, at_ms: i64) -> Self {
        self.expires_at = Some(at_ms);
        self
    }

    /// The tuple's identity: everything but the expiry, matching the
    /// `grants` primary key.
    #[must_use]
    pub const fn key(&self) -> (Object, TupleRelation, Subject) {
        (self.object, self.relation, self.subject)
    }

    /// Whether the tuple still counts at `now_ms`.
    #[must_use]
    pub fn is_live(&self, now_ms: i64) -> bool {
        self.expires_at.is_none_or(|at| at > now_ms)
    }

    /// Whether the tuple's parts fit together.
    #[must_use]
    pub const fn is_well_formed(&self) -> bool {
        matches!(
            (self.object, self.relation),
            (Object::Space(_) | Object::Node(_), TupleRelation::Role(_))
                | (Object::Group(_), TupleRelation::Member)
                | (Object::Node(_), TupleRelation::Restricted)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relations_imply_weaker_ones() {
        assert!(Relation::Owner.implies(Relation::Viewer));
        assert!(Relation::Editor.implies(Relation::Commenter));
        assert!(!Relation::Viewer.implies(Relation::Commenter));
        assert!(Relation::Manager.implies(Permission::Share.required()));
        assert!(!Relation::Editor.implies(Permission::Share.required()));
    }

    #[test]
    fn names_round_trip() {
        for r in Relation::ALL {
            assert_eq!(Relation::parse(r.as_str()), Some(r));
            assert_eq!(
                TupleRelation::parse(r.as_str()),
                Some(TupleRelation::Role(r))
            );
        }
        for t in [TupleRelation::Member, TupleRelation::Restricted] {
            assert_eq!(TupleRelation::parse(t.as_str()), Some(t));
        }
        assert_eq!(Relation::parse("admin"), None);
        let id = Id(42);
        for s in [
            Subject::User(id),
            Subject::Group(id),
            Subject::ShareLink(id),
        ] {
            let (k, i, r) = s.columns();
            assert_eq!(Subject::from_columns(k, i, r), Some(s));
        }
        assert_eq!(Subject::from_columns("group", id, ""), None);
        for o in [Object::Space(id), Object::Node(id), Object::Group(id)] {
            let (k, i) = o.columns();
            assert_eq!(Object::from_columns(k, i), Some(o));
        }
        assert_eq!(Object::from_columns("doc", id), None);
        assert_eq!(Id::from_bytes(Id(7).to_bytes()), Id(7));
        assert_eq!(Id(255).to_string(), "0000000000000000000000007Z");
        let doc = DocId::generate();
        assert_eq!(DocId::from(Id::from(doc)), doc);
        assert_eq!(
            Principal::from(UserId::from_u128(9)),
            Principal::User(Id(9))
        );
        assert_eq!(
            Principal::from(ShareId::from_u128(9)),
            Principal::ShareLink(Id(9))
        );
    }

    #[test]
    fn well_formed_tuples() {
        let u = Subject::User(Id(1));
        assert!(Tuple::role(Object::Node(Id(2)), Relation::Editor, u).is_well_formed());
        assert!(Tuple::member(Id(3), u).is_well_formed());
        assert!(Tuple::restricted(Id(2)).is_well_formed());
        let bad = Tuple {
            object: Object::Group(Id(3)),
            relation: TupleRelation::Role(Relation::Viewer),
            subject: u,
            expires_at: None,
        };
        assert!(!bad.is_well_formed());
    }

    #[test]
    fn expiry_is_exclusive() {
        let t =
            Tuple::role(Object::Space(Id(1)), Relation::Viewer, Subject::User(Id(2))).expiring(100);
        assert!(t.is_live(99));
        assert!(!t.is_live(100));
    }
}
