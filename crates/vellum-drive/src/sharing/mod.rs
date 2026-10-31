// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Sharing an item (#95, R-SE-3): grants to people and groups, and share
//! links.
//!
//! - A grant is an authorization tuple on the item (`viewer`, `commenter`,
//!   `editor` or `manager`, with an optional expiry). The engine checks
//!   that the person changing it may share the item.
//! - A share link opens one page read-only on the public origin. Its token
//!   is 22 base62 characters from the CSPRNG and only its SHA-256 hash is
//!   stored ([`vellum_kernel::ShareToken`]); the plaintext is shown once.
//!   The link is a `viewer` grant to `share_link:<id>`, so expiry and
//!   revocation are enforced by the same engine as every other access; the
//!   record here adds the password and the audit fields.
//!
//! [`Sharing`] works through three ports: [`ShareAccess`] for the grants,
//! [`ShareLinks`] for link records, and [`LinkPasswords`] for hashing
//! passwords (the binary passes the identity service's Argon2id).

pub mod sql;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use async_trait::async_trait;
use vellum_kernel::{
    Actor, ObjectId, ShareId, ShareToken, ShareTokenHash, SpaceId, Timestamp,
    UserId,
};

use crate::drive::Drive;
use crate::error::{DriveError, DriveResult};
use crate::model::{Node, NodeKind};

/// The longest password a link may carry, in bytes.
pub const MAX_PASSWORD_BYTES: usize = 256;
/// The shortest password a link may carry, in characters.
pub const MIN_PASSWORD_CHARS: usize = 8;
/// The most live links one item may have.
pub const MAX_LINKS_PER_ITEM: usize = 50;
/// Wrong passwords one link takes within [`PASSWORD_WINDOW_MS`] before it
/// stops checking passwords until the window passes (RT-042).
pub const MAX_PASSWORD_FAILURES: u32 = 10;
/// The window wrong passwords are counted in: 15 minutes.
pub const PASSWORD_WINDOW_MS: i64 = 15 * 60 * 1000;
/// Links whose failures are remembered at once; older counts are dropped
/// first so the map stays small under a flood.
const MAX_TRACKED_LINKS: usize = 10_000;

/// A role that can be granted on an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Read.
    Viewer,
    /// Read and comment.
    Commenter,
    /// Read, comment and edit.
    Editor,
    /// Everything an editor can do, plus share, move and trash.
    Manager,
    /// Everything, including handing ownership on. Listed, never granted
    /// here: ownership moves through its own transfer.
    Owner,
}

impl Role {
    /// The role's name, as the API spells it.
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

    /// Parses a role name.
    ///
    /// # Errors
    ///
    /// [`DriveError::Invalid`] for anything else, ownership included.
    pub fn parse(name: &str) -> DriveResult<Self> {
        match name {
            "viewer" => Ok(Self::Viewer),
            "commenter" => Ok(Self::Commenter),
            "editor" => Ok(Self::Editor),
            "manager" => Ok(Self::Manager),
            other => Err(DriveError::Invalid(format!(
                "role must be viewer, commenter, editor or manager, not \
                    {other:?}"
            ))),
        }
    }
}

/// Who a grant is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Grantee {
    /// One person.
    User(UserId),
    /// Every member of a group.
    Group(ObjectId),
}

impl Grantee {
    /// Parses `user:<ULID>` or `group:<ULID>`, the people picker's form.
    ///
    /// # Errors
    ///
    /// [`DriveError::Invalid`] for any other text.
    pub fn parse(text: &str) -> DriveResult<Self> {
        let bad = || {
            DriveError::Invalid(format!(
                "expected user:<id> or group:<id>, not {text:?}"
            ))
        };
        match text.split_once(':') {
            Some(("user", id)) => id.parse().map(Self::User).map_err(|_| bad()),
            Some(("group", id)) => {
                id.parse().map(Self::Group).map_err(|_| bad())
            }
            _ => Err(bad()),
        }
    }
}

impl fmt::Display for Grantee {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::User(id) => write!(f, "user:{id}"),
            Self::Group(id) => write!(f, "group:{id}"),
        }
    }
}

/// A role someone holds on an item through a grant made on that item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// Who.
    pub grantee: Grantee,
    /// Which role.
    pub role: Role,
    /// When it stops counting, if ever.
    pub expires_at: Option<Timestamp>,
}

/// A share link's stored record. The token itself is never stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRecord {
    /// The link's ID; its grant is to `share_link:<id>`.
    pub id: ShareId,
    /// The space that holds the page.
    pub space: SpaceId,
    /// The page it opens.
    pub node: ObjectId,
    /// SHA-256 of the token.
    pub token_hash: ShareTokenHash,
    /// An Argon2id PHC string when the link has a password.
    pub password_hash: Option<String>,
    /// When the link stops working, if ever.
    pub expires_at: Option<Timestamp>,
    /// Who made it.
    pub created_by: UserId,
    /// When it was made.
    pub created_at: Timestamp,
    /// When it was revoked; revoked links never open again.
    pub revoked_at: Option<Timestamp>,
}

impl LinkRecord {
    /// Whether the link opens at `now`.
    #[must_use]
    pub fn is_live(&self, now: Timestamp) -> bool {
        self.revoked_at.is_none()
            && self.expires_at.is_none_or(|t| t.unix_ms() > now.unix_ms())
    }
}

/// A link as its owners see it: the record without the hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInfo {
    /// The link's ID.
    pub id: ShareId,
    /// Whether it asks for a password.
    pub has_password: bool,
    /// When it stops working, if ever.
    pub expires_at: Option<Timestamp>,
    /// Who made it.
    pub created_by: UserId,
    /// When it was made.
    pub created_at: Timestamp,
    /// Whether it still opens.
    pub live: bool,
}

impl LinkInfo {
    fn of(record: &LinkRecord, now: Timestamp) -> Self {
        Self {
            id: record.id,
            has_password: record.password_hash.is_some(),
            expires_at: record.expires_at,
            created_by: record.created_by,
            created_at: record.created_at,
            live: record.is_live(now),
        }
    }
}

/// A new link, with its token: the only time the plaintext exists.
#[derive(Debug, Clone)]
pub struct NewLink {
    /// The link.
    pub info: LinkInfo,
    /// The token for `/s/<token>` on the public origin.
    pub token: ShareToken,
}

/// Who has access to an item, as its sharers see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharingState {
    /// The item.
    pub node: Node,
    /// Grants made on this item (inherited access is not repeated).
    pub grants: Vec<Grant>,
    /// Its share links, newest first, revoked ones included.
    pub links: Vec<LinkInfo>,
}

/// What opening a share link found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// The page the link opens.
    Page(Box<Node>),
    /// The link has a password; ask for it (again, when `retry`).
    PasswordRequired {
        /// Whether a wrong password was just given.
        retry: bool,
    },
}

/// Grants on items, through the authorization engine.
#[async_trait]
pub trait ShareAccess: Send + Sync + fmt::Debug {
    /// Whether `user` may change who has access to `node`.
    async fn can_share(
        &self,
        user: UserId,
        space: SpaceId,
        node: ObjectId,
    ) -> DriveResult<bool>;
    /// The role grants made directly on `node`, live ones only.
    async fn grants(
        &self,
        space: SpaceId,
        node: ObjectId,
    ) -> DriveResult<Vec<Grant>>;
    /// Grants `role` (replacing that grantee's other roles on the item);
    /// [`DriveError::PermissionDenied`] unless `user` may share it.
    async fn grant(
        &self,
        user: UserId,
        space: SpaceId,
        node: ObjectId,
        grant: &Grant,
    ) -> DriveResult<()>;
    /// Removes every role the grantee holds through grants on the item.
    async fn revoke(
        &self,
        user: UserId,
        space: SpaceId,
        node: ObjectId,
        grantee: Grantee,
    ) -> DriveResult<()>;
    /// Gives a link `viewer` on the page until `expires_at`.
    async fn grant_link(
        &self,
        user: UserId,
        space: SpaceId,
        node: ObjectId,
        link: ShareId,
        expires_at: Option<Timestamp>,
    ) -> DriveResult<()>;
    /// Takes a link's grant away.
    async fn revoke_link(
        &self,
        user: UserId,
        space: SpaceId,
        node: ObjectId,
        link: ShareId,
    ) -> DriveResult<()>;
    /// Whether the link's grant still lets it read the page.
    async fn link_can_read(
        &self,
        space: SpaceId,
        node: ObjectId,
        link: ShareId,
    ) -> DriveResult<bool>;
}

/// Where link records live.
#[async_trait]
pub trait ShareLinks: Send + Sync + fmt::Debug {
    /// Stores a new link.
    async fn insert(&self, record: &LinkRecord) -> DriveResult<()>;
    /// The links of an item, newest first.
    async fn of_node(&self, node: ObjectId) -> DriveResult<Vec<LinkRecord>>;
    /// One link by ID.
    async fn get(&self, id: ShareId) -> DriveResult<Option<LinkRecord>>;
    /// The link whose token hashes to `hash`.
    async fn by_hash(
        &self,
        hash: &ShareTokenHash,
    ) -> DriveResult<Option<LinkRecord>>;
    /// Marks a link revoked; a second call keeps the first time.
    async fn revoke(&self, id: ShareId, at: Timestamp) -> DriveResult<()>;
}

/// Hashes and checks link passwords. Both are deliberately slow, so an
/// implementation runs them off the async workers.
#[async_trait]
pub trait LinkPasswords: Send + Sync + fmt::Debug {
    /// A PHC string for the password.
    ///
    /// # Errors
    ///
    /// The hasher failed.
    async fn hash(&self, password: String) -> DriveResult<String>;
    /// Whether the password matches the stored PHC string.
    async fn verify(&self, password: String, stored: String) -> bool;
}

/// Link records in memory, for tests and `vellum openapi`.
#[derive(Debug, Default)]
pub struct MemoryShareLinks(Mutex<Vec<LinkRecord>>);

impl MemoryShareLinks {
    fn with<T>(&self, f: impl FnOnce(&mut Vec<LinkRecord>) -> T) -> T {
        f(&mut self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

#[async_trait]
impl ShareLinks for MemoryShareLinks {
    async fn insert(&self, record: &LinkRecord) -> DriveResult<()> {
        self.with(|all| {
            if all
                .iter()
                .any(|r| r.id == record.id || r.token_hash == record.token_hash)
            {
                return Err(DriveError::AlreadyExists(format!(
                    "share link {}",
                    record.id
                )));
            }
            all.push(record.clone());
            Ok(())
        })
    }

    async fn of_node(&self, node: ObjectId) -> DriveResult<Vec<LinkRecord>> {
        let mut found: Vec<LinkRecord> = self.with(|all| {
            all.iter().filter(|r| r.node == node).cloned().collect()
        });
        found
            .sort_by_key(|r| std::cmp::Reverse((r.created_at.unix_ms(), r.id)));
        Ok(found)
    }

    async fn get(&self, id: ShareId) -> DriveResult<Option<LinkRecord>> {
        Ok(self.with(|all| all.iter().find(|r| r.id == id).cloned()))
    }

    async fn by_hash(
        &self,
        hash: &ShareTokenHash,
    ) -> DriveResult<Option<LinkRecord>> {
        Ok(
            self.with(|all| {
                all.iter().find(|r| &r.token_hash == hash).cloned()
            }),
        )
    }

    async fn revoke(&self, id: ShareId, at: Timestamp) -> DriveResult<()> {
        self.with(|all| {
            if let Some(record) = all.iter_mut().find(|r| r.id == id) {
                record.revoked_at = record.revoked_at.or(Some(at));
            }
        });
        Ok(())
    }
}

/// Sharing over the drive (#95).
#[derive(Clone)]
pub struct Sharing {
    drive: Arc<Drive>,
    access: Arc<dyn ShareAccess>,
    links: Arc<dyn ShareLinks>,
    passwords: Arc<dyn LinkPasswords>,
    clock: Arc<dyn Fn() -> Timestamp + Send + Sync>,
    public_origin: Option<String>,
    /// Wrong passwords per link: how many, and when the count started.
    failures: Arc<Mutex<HashMap<ShareId, (u32, i64)>>>,
}

impl fmt::Debug for Sharing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sharing")
            .field("access", &self.access)
            .field("links", &self.links)
            .finish_non_exhaustive()
    }
}

/// What every unusable link answers, so the answer never says which.
const fn link_gone() -> DriveError {
    DriveError::NotFound(ObjectId::from_u128(0))
}

/// The signed-in person behind an actor.
const fn person(actor: &Actor) -> DriveResult<UserId> {
    match actor {
        Actor::User { user } => Ok(*user),
        _ => Err(DriveError::PermissionDenied),
    }
}

impl Sharing {
    /// Sharing over `drive`.
    #[must_use]
    pub fn new(
        drive: Arc<Drive>,
        access: Arc<dyn ShareAccess>,
        links: Arc<dyn ShareLinks>,
        passwords: Arc<dyn LinkPasswords>,
    ) -> Self {
        Self {
            drive,
            access,
            links,
            passwords,
            clock: Arc::new(Timestamp::now),
            public_origin: None,
            failures: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Names the public origin share links open on
    /// (`https://pages.northstar.dev`); without one, links cannot be made.
    #[must_use]
    pub fn with_public_origin(mut self, origin: Option<String>) -> Self {
        self.public_origin = origin.map(|o| o.trim_end_matches('/').to_owned());
        self
    }

    /// The public origin, if configured.
    #[must_use]
    pub fn public_origin(&self) -> Option<&str> {
        self.public_origin.as_deref()
    }

    /// Replaces the clock, for tests.
    #[must_use]
    pub fn with_clock(
        mut self,
        clock: Arc<dyn Fn() -> Timestamp + Send + Sync>,
    ) -> Self {
        self.clock = clock;
        self
    }

    fn now(&self) -> Timestamp {
        (self.clock)()
    }

    /// Whether `link` has had too many wrong passwords lately. Counts are
    /// per replica: a cluster of `n` allows `n` times as many guesses,
    /// still far too few for a guessed password of 8 characters or more.
    fn locked_out(&self, link: ShareId) -> bool {
        let now = self.now().unix_ms();
        let failures =
            self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        failures.get(&link).is_some_and(|(count, since)| {
            *count >= MAX_PASSWORD_FAILURES && now - since < PASSWORD_WINDOW_MS
        })
    }

    /// Counts a wrong password for `link`.
    fn failed(&self, link: ShareId) {
        let now = self.now().unix_ms();
        let mut failures =
            self.failures.lock().unwrap_or_else(PoisonError::into_inner);
        if failures.len() >= MAX_TRACKED_LINKS {
            failures.retain(|_, (_, since)| now - *since < PASSWORD_WINDOW_MS);
            if failures.len() >= MAX_TRACKED_LINKS {
                failures.clear();
            }
        }
        let entry = failures.entry(link).or_insert((0, now));
        if now - entry.1 >= PASSWORD_WINDOW_MS {
            *entry = (0, now);
        }
        entry.0 = entry.0.saturating_add(1);
        drop(failures);
    }

    /// The live item, if `user` may share it. Someone who may only view it
    /// gets [`DriveError::PermissionDenied`]; someone who may not even see
    /// it gets [`DriveError::NotFound`], as the drive answers elsewhere.
    async fn shareable(
        &self,
        actor: &Actor,
        id: ObjectId,
    ) -> DriveResult<(UserId, Node)> {
        let user = person(actor)?;
        let node = self.drive.get(actor, id).await?;
        if !node.is_live() {
            return Err(DriveError::NotFound(id));
        }
        if !self.access.can_share(user, node.space, node.id).await? {
            return Err(DriveError::PermissionDenied);
        }
        Ok((user, node))
    }

    /// Who has access to an item through grants on it, and its links.
    ///
    /// # Errors
    ///
    /// `NotFound`, `PermissionDenied` (the caller may not share it), or a
    /// storage failure.
    pub async fn state(
        &self,
        actor: &Actor,
        id: ObjectId,
    ) -> DriveResult<SharingState> {
        let (_, node) = self.shareable(actor, id).await?;
        let mut grants = self.access.grants(node.space, node.id).await?;
        grants.sort_by_key(|g| (std::cmp::Reverse(g.role), g.grantee));
        let now = self.now();
        let links = self
            .links
            .of_node(node.id)
            .await?
            .iter()
            .map(|r| LinkInfo::of(r, now))
            .collect();
        Ok(SharingState {
            node,
            grants,
            links,
        })
    }

    /// Grants a role on an item, replacing the grantee's other roles there.
    ///
    /// # Errors
    ///
    /// `NotFound`, `PermissionDenied`, `Invalid` for an expiry in the past,
    /// or a storage failure.
    pub async fn share(
        &self,
        actor: &Actor,
        id: ObjectId,
        grant: &Grant,
    ) -> DriveResult<()> {
        let (user, node) = self.shareable(actor, id).await?;
        if let Some(at) = grant.expires_at
            && at.unix_ms() <= self.now().unix_ms()
        {
            return Err(DriveError::Invalid(
                "the expiry must be in the future".into(),
            ));
        }
        if grant.grantee == Grantee::User(user) {
            return Err(DriveError::Invalid(
                "you cannot change your own access".into(),
            ));
        }
        if grant.role == Role::Owner {
            return Err(DriveError::Invalid(
                "ownership cannot be granted here".into(),
            ));
        }
        self.access.grant(user, node.space, node.id, grant).await?;
        tracing::info!(
            node = %node.id,
            grantee = %grant.grantee,
            role = grant.role.as_str(),
            "item shared"
        );
        Ok(())
    }

    /// Removes the roles a grantee holds through grants on an item.
    ///
    /// # Errors
    ///
    /// `NotFound`, `PermissionDenied`, or a storage failure.
    pub async fn unshare(
        &self,
        actor: &Actor,
        id: ObjectId,
        grantee: Grantee,
    ) -> DriveResult<()> {
        let (user, node) = self.shareable(actor, id).await?;
        if grantee == Grantee::User(user) {
            return Err(DriveError::Invalid(
                "you cannot change your own access".into(),
            ));
        }
        self.access
            .revoke(user, node.space, node.id, grantee)
            .await?;
        tracing::info!(node = %node.id, grantee = %grantee, "item unshared");
        Ok(())
    }

    /// Makes a share link to a page. The token in the answer is the only
    /// copy: it cannot be shown again.
    ///
    /// # Errors
    ///
    /// `NotFound`, `PermissionDenied`, `Invalid` (not a page, a past expiry,
    /// a password too short or too long, too many links), or a storage
    /// failure.
    pub async fn create_link(
        &self,
        actor: &Actor,
        id: ObjectId,
        expires_at: Option<Timestamp>,
        password: Option<&str>,
    ) -> DriveResult<NewLink> {
        let (user, node) = self.shareable(actor, id).await?;
        if node.kind != NodeKind::Page {
            return Err(DriveError::Invalid(
                "only a page can have a share link".into(),
            ));
        }
        let now = self.now();
        if expires_at.is_some_and(|t| t.unix_ms() <= now.unix_ms()) {
            return Err(DriveError::Invalid(
                "the expiry must be in the future".into(),
            ));
        }
        let password_hash = match password {
            None => None,
            Some(p) if p.len() > MAX_PASSWORD_BYTES => {
                return Err(DriveError::Invalid(format!(
                    "the password must be at most {MAX_PASSWORD_BYTES} bytes"
                )));
            }
            Some(p) if p.chars().count() < MIN_PASSWORD_CHARS => {
                return Err(DriveError::Invalid(format!(
                    "the password must be at least {MIN_PASSWORD_CHARS} \
                        characters"
                )));
            }
            Some(p) => Some(self.passwords.hash(p.to_owned()).await?),
        };
        let live = self
            .links
            .of_node(node.id)
            .await?
            .iter()
            .filter(|r| r.is_live(now))
            .count();
        if live >= MAX_LINKS_PER_ITEM {
            return Err(DriveError::Invalid(format!(
                "a page can have at most {MAX_LINKS_PER_ITEM} live links; \
                    revoke one first"
            )));
        }
        let token = ShareToken::generate().map_err(|e| {
            DriveError::Index(format!("cannot make a token: {e}"))
        })?;
        let record = LinkRecord {
            id: ShareId::generate(),
            space: node.space,
            node: node.id,
            token_hash: token.hash(),
            password_hash,
            expires_at,
            created_by: user,
            created_at: now,
            revoked_at: None,
        };
        // The record first: a grant without a record can never be opened,
        // while a record without its grant reads as revoked.
        self.links.insert(&record).await?;
        if let Err(error) = self
            .access
            .grant_link(user, node.space, node.id, record.id, expires_at)
            .await
        {
            self.links.revoke(record.id, now).await?;
            return Err(error);
        }
        tracing::info!(
            node = %node.id,
            link = %record.id,
            password = record.password_hash.is_some(),
            "share link created"
        );
        Ok(NewLink {
            info: LinkInfo::of(&record, now),
            token,
        })
    }

    /// Revokes a link: it never opens again.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown link, `PermissionDenied` when the caller
    /// may not share its page, or a storage failure.
    pub async fn revoke_link(
        &self,
        actor: &Actor,
        link: ShareId,
    ) -> DriveResult<()> {
        let record = self.links.get(link).await?.ok_or_else(link_gone)?;
        let (user, node) = self.shareable(actor, record.node).await?;
        self.links.revoke(link, self.now()).await?;
        self.access
            .revoke_link(user, node.space, node.id, link)
            .await?;
        tracing::info!(node = %node.id, link = %link, "share link revoked");
        Ok(())
    }

    /// Opens a share link for an anonymous visitor. Unknown, expired and
    /// revoked links, and links whose page is gone, are all `NotFound`, so
    /// the answer never says which.
    ///
    /// # Errors
    ///
    /// `NotFound`, or a storage failure.
    pub async fn open(
        &self,
        token: &ShareToken,
        password: Option<&str>,
    ) -> DriveResult<Opened> {
        let gone = link_gone;
        let record =
            self.links.by_hash(&token.hash()).await?.ok_or_else(gone)?;
        if !record.token_hash.verify(token) || !record.is_live(self.now()) {
            return Err(gone());
        }
        if let Some(stored) = &record.password_hash {
            let Some(given) = password else {
                return Ok(Opened::PasswordRequired { retry: false });
            };
            // Too many wrong guesses: refuse without hashing, so neither a
            // guessing run nor a flood of slow hashes gets anywhere (RT-042).
            if self.locked_out(record.id) {
                tracing::warn!(link = %record.id, "share link password locked \
                    after repeated failures");
                return Ok(Opened::PasswordRequired { retry: true });
            }
            if given.len() > MAX_PASSWORD_BYTES
                || !self
                    .passwords
                    .verify(given.to_owned(), stored.clone())
                    .await
            {
                self.failed(record.id);
                tracing::info!(
                    link = %record.id,
                    "share link password refused"
                );
                return Ok(Opened::PasswordRequired { retry: true });
            }
            let _ = self
                .failures
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&record.id);
        }
        if !self
            .access
            .link_can_read(record.space, record.node, record.id)
            .await?
        {
            return Err(gone());
        }
        let node = self
            .drive
            .get(&Actor::System, record.node)
            .await
            .map_err(|_| gone())?;
        if !node.is_live() || node.kind != NodeKind::Page {
            return Err(gone());
        }
        Ok(Opened::Page(Box::new(node)))
    }

    /// The drive, for reading what a link opened.
    #[must_use]
    pub const fn drive(&self) -> &Arc<Drive> {
        &self.drive
    }
}
