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
use vellum_kernel::{Error, ErrorKind, ObjectId, Timestamp, UserId};

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
    /// Authenticator apps, passkeys and recovery codes (D290).
    #[serde(default, skip_serializing_if = "SecondFactors::is_empty")]
    pub factors: SecondFactors,
    /// A break-glass admin: may still sign in with a password and a second
    /// factor when single sign-on is required (D292).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub break_glass: bool,
}

/// A person's second factors (ADR 0069, D290).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SecondFactors {
    /// The authenticator app, once confirmed.
    pub totp: Option<TotpFactor>,
    /// An authenticator app being added, until its first code is entered.
    pub pending_totp: Option<PendingTotp>,
    /// Registered passkeys.
    pub passkeys: Vec<Passkey>,
    /// Hashes of the recovery codes not used yet.
    pub recovery: Vec<String>,
}

impl SecondFactors {
    /// Whether nothing is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Whether sign-in needs a second step: a confirmed app or a passkey.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.totp.is_some() || !self.passkeys.is_empty()
    }
}

/// A confirmed authenticator app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotpFactor {
    /// The secret, sealed with the identity key ([`crate::seal`]).
    pub sealed: String,
    /// The last step a code was accepted for; earlier steps are refused.
    pub last_step: Option<u64>,
    /// When it was added.
    pub added_at: Timestamp,
}

/// An authenticator app waiting for its first code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingTotp {
    /// The secret, sealed.
    pub sealed: String,
    /// When the offer lapses.
    pub expires: Timestamp,
}

/// A registered passkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Passkey {
    /// The credential ID, base64url.
    pub id: String,
    /// The owner's label for it.
    pub name: String,
    /// The public key.
    pub key: crate::webauthn::PublicKey,
    /// The signature counter last seen.
    pub sign_count: u32,
    /// When it was added.
    pub created_at: Timestamp,
    /// When it last signed in.
    pub last_used_at: Option<Timestamp>,
}

/// What a second-factor challenge is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengePurpose {
    /// The second step of a password sign-in.
    SignIn,
    /// A sign-in that may finish only once a factor is added.
    Enrol,
    /// Adding a passkey from the settings page.
    Register,
    /// Confirming a session's owner with a passkey.
    Reauth,
}

/// A second-factor challenge in flight: a sign-in ticket or a passkey
/// ceremony, keyed by the hash of its secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MfaChallenge {
    /// Whose it is.
    pub user: UserId,
    /// What it is for.
    pub purpose: ChallengePurpose,
    /// The `WebAuthn` challenge issued for it, base64url.
    pub webauthn: Option<String>,
    /// Wrong answers so far; the ticket ends after a few.
    pub failures: u32,
    /// When it lapses.
    pub expires: Timestamp,
    /// The browser that started it.
    pub user_agent: Option<String>,
    /// For [`ChallengePurpose::Reauth`], the session it confirms.
    pub session: Option<String>,
}

/// A named group of people (ADR 0057, D273). Who is in it lives in the
/// permission engine; this is its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// The group's ID; grants name it as `group:<ID>`.
    pub id: ObjectId,
    /// The display name, unique ignoring case.
    pub name: String,
    /// What the group is for.
    #[serde(default)]
    pub description: String,
    /// When it was created.
    pub created_at: Timestamp,
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
    /// When the user last proved their identity before approving it: the
    /// session's sign-in time, not the approval's. Tokens of the grant
    /// count as signed in then, so approving a client from an old session
    /// never yields a credential that passes a recent-sign-in check
    /// (RT-039). Grants from before this field use `created_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_time: Option<Timestamp>,
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
    /// When the approving session last proved the user's identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_time: Option<Timestamp>,
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
    /// Named groups by ID.
    #[serde(default)]
    pub groups: BTreeMap<String, Group>,
    /// How people may sign in, set in Admin → Sign-in (ADR 0069).
    #[serde(default)]
    pub sign_in: SignInSettings,
    /// Second-factor challenges by the base64 of their secret's hash.
    #[serde(default)]
    pub mfa_challenges: BTreeMap<String, MfaChallenge>,
    /// SAML sign-ins in progress by the base64 of their relay state's hash.
    #[serde(default)]
    pub saml_requests: BTreeMap<String, SamlRequest>,
    /// Assertions already used, by the base64 of the hash of provider and
    /// assertion ID, until they expire (replay protection).
    #[serde(default)]
    pub saml_seen: BTreeMap<String, Timestamp>,
}

/// The workspace's sign-in rules, one record set by administrators
/// (ADR 0069).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SignInSettings {
    /// The longest expiry, in days, a personal API token may have; `None`
    /// allows tokens that never expire (D291).
    pub max_token_days: Option<u32>,
    /// Who must use a second factor with a password (D290).
    pub second_factor: SecondFactorPolicy,
    /// Whether passwords are refused, except for break-glass admins (D292).
    pub require_sso: bool,
    /// Identity providers added in Admin → Sign-in (D292).
    pub providers: Vec<Provider>,
}

/// An identity provider people sign in with (ADR 0069, D292).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provider {
    /// A short, stable ID (`okta`, `google-2`), used in URLs.
    pub id: String,
    /// The button label: "Sign in with {name}".
    pub name: String,
    /// Whether the sign-in page offers it.
    pub enabled: bool,
    /// Email domains whose people get an account on first sign-in.
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// Whether a verified address with no account gets one.
    #[serde(default)]
    pub auto_create: bool,
    /// When it was added.
    pub created_at: Timestamp,
    /// The protocol and its settings.
    pub protocol: Protocol,
}

/// How Vellum talks to a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Protocol {
    /// `OpenID Connect`, authorization code flow with PKCE.
    Oidc {
        /// The issuer URL.
        issuer: String,
        /// The client ID registered with the provider.
        client_id: String,
        /// The client secret, sealed ([`crate::seal`]); `None` for a
        /// public client.
        client_secret: Option<String>,
    },
    /// SAML 2.0, Vellum as the service provider (D293).
    Saml {
        /// The provider's entity ID, as its assertions name their issuer.
        entity_id: String,
        /// Where sign-in requests go (HTTP-Redirect binding).
        sso_url: String,
        /// The signing certificates, base64 DER.
        certificates: Vec<String>,
        /// The metadata URL the settings were imported from, if any.
        #[serde(default)]
        metadata_url: Option<String>,
    },
}

/// A SAML sign-in in progress, by the hash of its relay state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamlRequest {
    /// The provider it went to.
    pub provider: String,
    /// The `AuthnRequest` ID the response must answer.
    pub request_id: String,
    /// Where to go afterwards (a local path).
    pub next: String,
    /// When it lapses.
    pub expires: Timestamp,
}

/// Who must use a second factor when signing in with a password.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecondFactorPolicy {
    /// Nobody: people choose.
    #[default]
    Optional,
    /// Administrators.
    Admins,
    /// Everyone.
    Everyone,
}

impl SecondFactorPolicy {
    /// Whether the policy covers `user`.
    #[must_use]
    pub const fn covers(self, user: &User) -> bool {
        match self {
            Self::Optional => false,
            Self::Admins => user.admin,
            Self::Everyone => true,
        }
    }
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
        self.mfa_challenges.retain(|_, c| c.expires > now);
        self.saml_requests.retain(|_, r| r.expires > now);
        self.saml_seen.retain(|_, until| *until > now);
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
        self.mfa_challenges.retain(|_, c| c.user != user);
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
                        factors: SecondFactors::default(),
                        break_glass: false,
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
