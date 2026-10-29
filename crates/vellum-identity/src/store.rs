// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The identity records and where they are kept.
//!
//! Every record lives in one [`State`]. Where the state is kept depends on
//! how the server runs:
//!
//! - **The single binary** keeps it in memory and writes it after each
//!   change to `.vellum/identity.json` in the data directory (mode 0600,
//!   replaced atomically).
//! - **Several replicas** keep it in the index database
//!   ([`Store::shared`], ADR 0037, D160), so a sign-in, a revoked session
//!   or a lockout on one replica holds on every other.
//!
//! Secrets appear only as SHA-256 hashes; passwords only as Argon2id
//! hashes; lockout keys only as SHA-256 hashes of the address.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_contracts::Scope;
use vellum_kernel::{Error, ErrorKind, Timestamp, UserId};

use crate::oauth::Client;
use crate::secret::SecretHash;
use crate::shared::Shared;

/// How long a lockout record outlives the end of its lockout.
const LOCKOUT_MEMORY: std::time::Duration = std::time::Duration::from_mins(15);
/// At most this many lockout records are kept; beyond it, the half that
/// unlocks soonest is dropped (the attacker's own heavily locked keys go
/// last).
const MAX_LOCKOUTS: usize = 10_000;

/// A person who can sign in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    /// The user's ID.
    pub id: UserId,
    /// The sign-in address, lower-cased.
    pub email: String,
    /// The display name.
    pub name: String,
    /// A unique, stable handle (`priya`, `priya-2`) that Work files use
    /// for assignees and mentions. Never changes once assigned.
    #[serde(default)]
    pub handle: String,
    /// The Argon2id hash; `None` for accounts that sign in only by SSO.
    pub password_hash: Option<String>,
    /// Whether the user administers the workspace.
    pub admin: bool,
    /// Whether sign-in is blocked. Disabling revokes every credential.
    pub disabled: bool,
    /// When the account was created.
    pub created_at: Timestamp,
    /// Single sign-on identities that sign in as this user.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sso: Vec<SsoLink>,
}

/// A provider's user, linked to a Vellum account on first sign-in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SsoLink {
    /// The provider's issuer URL.
    pub issuer: String,
    /// The provider's stable user ID (`sub`).
    pub subject: String,
}

/// How a browser session was established.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionMethod {
    /// Email and password.
    Password,
    /// A one-time break-glass code.
    BreakGlass,
    /// Single sign-on.
    Sso,
}

/// A browser session, identified by its cookie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// A stable public ID (a label of the hash), for listing and revoking.
    pub id: String,
    /// The hash of the cookie value.
    #[serde(with = "hash_b64")]
    pub hash: SecretHash,
    /// The user.
    pub user: UserId,
    /// How it was established.
    pub method: SessionMethod,
    /// When it was created.
    pub created_at: Timestamp,
    /// When it was last used (updated at most every few minutes).
    pub last_seen_at: Timestamp,
    /// When the user last proved their identity.
    pub authenticated_at: Timestamp,
    /// When it ends regardless of use.
    pub expires_at: Timestamp,
    /// The browser's `User-Agent`, to help users recognise it.
    pub user_agent: Option<String>,
}

/// A personal API token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiToken {
    /// A stable public ID.
    pub id: String,
    /// The hash of the token.
    #[serde(with = "hash_b64")]
    pub hash: SecretHash,
    /// The owner.
    pub user: UserId,
    /// The owner's label for it.
    pub name: String,
    /// The granted scopes, as chosen (not expanded).
    pub scopes: Vec<Scope>,
    /// When it was created.
    pub created_at: Timestamp,
    /// When it stops working, if ever.
    pub expires_at: Option<Timestamp>,
    /// When it was last used (updated at most every few minutes).
    pub last_used_at: Option<Timestamp>,
}

/// An `OAuth` grant: one client's access on one user's behalf. Refresh
/// tokens rotate within the grant; presenting an old one revokes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// A stable public ID.
    pub id: String,
    /// The user.
    pub user: UserId,
    /// The `OAuth` client ID.
    pub client_id: String,
    /// The client's display name.
    pub client_name: String,
    /// The granted scopes.
    pub scopes: Vec<Scope>,
    /// The resources (RFC 8707) the tokens are for.
    pub audience: Vec<String>,
    /// When the user approved it.
    pub created_at: Timestamp,
    /// The current refresh token's hash.
    #[serde(with = "hash_b64")]
    pub refresh_hash: SecretHash,
    /// Hashes of refresh tokens already rotated out, to detect reuse.
    #[serde(with = "hashes_b64")]
    pub spent_refresh: Vec<SecretHash>,
    /// When the current refresh token expires.
    pub refresh_expires_at: Timestamp,
}

/// An `OAuth` access token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessToken {
    /// The grant it belongs to.
    pub grant: String,
    /// The scopes of this token: the grant's, or fewer after a narrowing
    /// refresh.
    pub scopes: Vec<Scope>,
    /// The resources this token is for.
    pub audience: Vec<String>,
    /// When it expires.
    pub expires_at: Timestamp,
}

/// Failed sign-in attempts against one address or client IP, shared by
/// every replica.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempts {
    /// Failures since the last success.
    pub failures: u32,
    /// When the next attempt is allowed.
    pub until: Timestamp,
}

/// An `OAuth` consent screen waiting for the user's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingConsent {
    /// The signed-in user who was asked.
    pub user: UserId,
    /// The client asking.
    pub client: Client,
    /// Where to send the browser back.
    pub redirect_uri: String,
    /// The client's `state`, returned unchanged.
    pub state: Option<String>,
    /// The scopes asked for, within the user's ceiling.
    pub scopes: Vec<Scope>,
    /// The resources (RFC 8707) the tokens will be for.
    pub audience: Vec<String>,
    /// The PKCE challenge.
    pub challenge: String,
    /// When the screen stops working.
    pub expires: Timestamp,
}

/// An `OAuth` authorization code waiting to be exchanged, once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingCode {
    /// The user who approved.
    pub user: UserId,
    /// The client it was issued to.
    pub client: Client,
    /// The redirect URI the exchange must repeat.
    pub redirect_uri: String,
    /// The approved scopes.
    pub scopes: Vec<Scope>,
    /// The resources the tokens will be for.
    pub audience: Vec<String>,
    /// The PKCE challenge.
    pub challenge: String,
    /// When the code stops working.
    pub expires: Timestamp,
}

/// Every identity record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Users by ID.
    pub users: BTreeMap<UserId, User>,
    /// Sessions by public ID.
    pub sessions: BTreeMap<String, Session>,
    /// API tokens by public ID.
    pub api_tokens: BTreeMap<String, ApiToken>,
    /// `OAuth` grants by public ID.
    pub grants: BTreeMap<String, Grant>,
    /// `OAuth` access tokens by the base64 of their hash.
    pub access_tokens: BTreeMap<String, AccessToken>,
    /// Lockout counters by the hex SHA-256 of `email:<address>` or
    /// `ip:<address>`. Only filled when the store is shared; a single
    /// process counts in memory.
    #[serde(default)]
    pub lockouts: BTreeMap<String, Attempts>,
    /// `OAuth` consent screens by the base64 of their ID's hash.
    #[serde(default)]
    pub oauth_consents: BTreeMap<String, PendingConsent>,
    /// `OAuth` authorization codes by the base64 of their hash.
    #[serde(default)]
    pub oauth_codes: BTreeMap<String, PendingCode>,
}

impl State {
    /// Finds a user by sign-in address.
    #[must_use]
    pub fn user_by_email(&self, email: &str) -> Option<&User> {
        self.users.values().find(|u| u.email == email)
    }

    /// A free handle derived from an email address: the local part,
    /// lower-cased, with characters outside `a-z 0-9 . _ -` replaced by
    /// `-`, at most 60 characters, and `-2`, `-3`... appended on collision.
    #[must_use]
    pub fn free_handle(&self, email: &str) -> String {
        let local = email.split('@').next().unwrap_or_default().to_lowercase();
        let mut base: String = local
            .chars()
            .map(|c| {
                if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '-'
                }
            })
            .take(60)
            .collect();
        if !base.starts_with(|c: char| c.is_ascii_alphanumeric()) {
            base.insert(0, 'u');
        }
        let taken = |h: &str| self.users.values().any(|u| u.handle == h);
        if !taken(&base) {
            return base;
        }
        (2..=u32::MAX)
            .map(|n| format!("{base}-{n}"))
            .find(|h| !taken(h))
            .unwrap_or(base)
    }

    /// Finds a user by handle.
    #[must_use]
    pub fn user_by_handle(&self, handle: &str) -> Option<&User> {
        self.users.values().find(|u| u.handle == handle)
    }

    /// Finds a session by the hash of its cookie.
    #[must_use]
    pub fn session_by_hash(&self, hash: &SecretHash) -> Option<&Session> {
        self.sessions.get(&crate::secret::label(hash))
    }

    /// Drops every expired record.
    pub fn sweep(&mut self, now: Timestamp) {
        self.sessions.retain(|_, s| s.expires_at > now);
        self.api_tokens
            .retain(|_, t| t.expires_at.is_none_or(|at| at > now));
        self.grants.retain(|_, g| g.refresh_expires_at > now);
        let grants = &self.grants;
        self.access_tokens
            .retain(|_, t| t.expires_at > now && grants.contains_key(&t.grant));
        self.oauth_consents.retain(|_, c| c.expires > now);
        self.oauth_codes.retain(|_, c| c.expires > now);
        let forget = i64::try_from(LOCKOUT_MEMORY.as_millis()).unwrap_or(i64::MAX);
        self.lockouts
            .retain(|_, a| a.until.unix_ms().saturating_add(forget) > now.unix_ms());
    }

    /// Makes room for one more lockout record.
    pub fn bound_lockouts(&mut self, now: Timestamp) {
        if self.lockouts.len() < MAX_LOCKOUTS {
            return;
        }
        self.sweep(now);
        if self.lockouts.len() >= MAX_LOCKOUTS {
            let mut untils: Vec<i64> = self.lockouts.values().map(|a| a.until.unix_ms()).collect();
            untils.sort_unstable();
            if let Some(&cutoff) = untils.get(untils.len() / 2) {
                self.lockouts.retain(|_, a| a.until.unix_ms() > cutoff);
            }
        }
    }

    /// Revokes every credential of a user.
    pub fn revoke_user(&mut self, user: UserId) {
        self.sessions.retain(|_, s| s.user != user);
        self.api_tokens.retain(|_, t| t.user != user);
        self.grants.retain(|_, g| g.user != user);
        let grants = &self.grants;
        self.access_tokens
            .retain(|_, t| grants.contains_key(&t.grant));
    }
}

/// The records: in memory and optionally a file, or in the index database.
#[derive(Debug)]
pub struct Store(Backend);

#[derive(Debug)]
enum Backend {
    Local {
        path: Option<PathBuf>,
        state: Mutex<State>,
    },
    Shared(Shared),
}

impl Store {
    /// A store that lives only in memory, for tests and demos.
    #[must_use]
    pub fn memory() -> Self {
        Self(Backend::Local {
            path: None,
            state: Mutex::new(State::default()),
        })
    }

    /// A store in the index database at `config`, shared by every replica
    /// that opens the same database. Opening applies pending migrations.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unavailable`] if the database cannot be opened or read.
    pub fn shared(config: vellum_db::IndexConfig) -> Result<Self, Error> {
        Ok(Self(Backend::Shared(Shared::open(config)?)))
    }

    /// Whether replicas share these records (and so must share lockouts).
    #[must_use]
    pub const fn is_shared(&self) -> bool {
        matches!(self.0, Backend::Shared(_))
    }

    /// Moves the records of a single-binary install into this store, once:
    /// when the store has no users and `<data_dir>/.vellum/identity.json`
    /// exists, its records are copied in and the file is renamed to
    /// `identity.json.imported`. Returns whether anything was imported.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the file cannot be read, parsed or
    /// renamed, and the errors of [`Store::write`].
    pub fn import_file(&self, data_dir: &Path) -> Result<bool, Error> {
        let file = Self::open(data_dir)?;
        let imported = file.read(Clone::clone);
        if imported.users.is_empty() {
            return Ok(false);
        }
        let copied = self.write(|s| {
            if s.users.is_empty() {
                *s = imported;
                true
            } else {
                false
            }
        })?;
        if copied {
            let path = data_dir.join(".vellum").join("identity.json");
            std::fs::rename(&path, path.with_extension("json.imported")).map_err(|e| {
                Error::new(
                    ErrorKind::Internal,
                    format!("cannot rename {}: {e}", path.display()),
                )
            })?;
        }
        Ok(copied)
    }

    /// Opens (or starts) the store file `<data_dir>/.vellum/identity.json`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the file exists but cannot be read or
    /// parsed.
    pub fn open(data_dir: &Path) -> Result<Self, Error> {
        let path = data_dir.join(".vellum").join("identity.json");
        let state = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                Error::new(
                    ErrorKind::Internal,
                    format!("cannot parse {}: {e}", path.display()),
                )
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => {
                return Err(Error::new(
                    ErrorKind::Internal,
                    format!("cannot read {}: {e}", path.display()),
                ));
            }
        };
        Ok(Self(Backend::Local {
            path: Some(path),
            state: Mutex::new(state),
        }))
    }

    /// Reads the records. A shared store reads the database's current
    /// records (falling back to the last ones seen if it is unreachable).
    pub fn read<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        match &self.0 {
            Backend::Local { state, .. } => {
                f(&state.lock().unwrap_or_else(PoisonError::into_inner))
            }
            Backend::Shared(shared) => shared.read(f),
        }
    }

    /// Changes the records and saves them.
    ///
    /// A file store keeps the change in memory even when saving fails, so a
    /// full disk degrades to memory-only rather than refusing every
    /// sign-in; the error is returned. A shared store keeps nothing when
    /// the database refuses the change, so replicas never disagree.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the file cannot be written, and
    /// [`ErrorKind::Unavailable`] if the database cannot be.
    pub fn write<R>(&self, f: impl FnOnce(&mut State) -> R) -> Result<R, Error> {
        match &self.0 {
            Backend::Local { path, state } => {
                let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                let result = f(&mut state);
                if let Some(path) = path {
                    let bytes = serde_json::to_vec(&*state).map_err(|e| {
                        Error::new(ErrorKind::Internal, format!("encode identity: {e}"))
                    })?;
                    drop(state);
                    save(path, &bytes)?;
                }
                Ok(result)
            }
            Backend::Shared(shared) => shared.write(f),
        }
    }
}

fn save(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let fail = |e: std::io::Error| {
        Error::new(
            ErrorKind::Internal,
            format!("cannot write {}: {e}", path.display()),
        )
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    let _ = options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let _ = options.mode(0o600);
    }
    let mut file = options.open(&tmp).map_err(fail)?;
    file.write_all(bytes).map_err(fail)?;
    file.sync_all().map_err(fail)?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(fail)
}

/// Encodes a secret hash for a map key.
#[must_use]
pub fn hash_key(hash: &SecretHash) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
}

mod hash_b64 {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde::{Deserialize as _, Deserializer, Serializer};

    use crate::secret::SecretHash;

    pub(super) fn serialize<S: Serializer>(hash: &SecretHash, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&URL_SAFE_NO_PAD.encode(hash))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<SecretHash, D::Error> {
        let text = String::deserialize(d)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(text)
            .map_err(serde::de::Error::custom)?;
        bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("a hash is 32 bytes"))
    }
}

mod hashes_b64 {
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde::{Deserialize as _, Deserializer, Serializer};

    use crate::secret::SecretHash;

    pub(super) fn serialize<S: Serializer>(hashes: &[SecretHash], s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(hashes.iter().map(|h| URL_SAFE_NO_PAD.encode(h)))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Vec<SecretHash>, D::Error> {
        Vec::<String>::deserialize(d)?
            .into_iter()
            .map(|text| {
                let bytes = URL_SAFE_NO_PAD
                    .decode(text)
                    .map_err(serde::de::Error::custom)?;
                bytes
                    .try_into()
                    .map_err(|_| serde::de::Error::custom("a hash is 32 bytes"))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_round_trips_and_is_private() {
        let dir = std::env::temp_dir().join(format!("vellum-identity-{}", std::process::id()));
        let store = Store::open(&dir).unwrap();
        let id = UserId::generate();
        store
            .write(|s| {
                let _ = s.users.insert(
                    id,
                    User {
                        id,
                        email: "priya@northstar.dev".into(),
                        name: "Priya".into(),
                        handle: "priya".into(),
                        password_hash: None,
                        admin: true,
                        disabled: false,
                        created_at: Timestamp::now(),
                        sso: Vec::new(),
                    },
                );
                let _ = s.sessions.insert(
                    "x".into(),
                    Session {
                        id: "x".into(),
                        hash: [7; 32],
                        user: id,
                        method: SessionMethod::Password,
                        created_at: Timestamp::now(),
                        last_seen_at: Timestamp::now(),
                        authenticated_at: Timestamp::now(),
                        expires_at: Timestamp::now(),
                        user_agent: None,
                    },
                );
            })
            .unwrap();
        assert_eq!(store.read(|s| s.free_handle("Priya@other.dev")), "priya-2");
        assert_eq!(store.read(|s| s.free_handle("Ann+Lee@x.dev")), "ann-lee");
        assert_eq!(store.read(|s| s.free_handle("_x@x.dev")), "u_x");
        let reopened = Store::open(&dir).unwrap();
        assert_eq!(reopened.read(Clone::clone), store.read(Clone::clone));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.join(".vellum/identity.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
