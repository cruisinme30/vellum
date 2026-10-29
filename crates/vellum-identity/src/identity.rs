// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Accounts, sessions, API tokens, lockout and break-glass access.

use std::collections::BTreeMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq as _;
use url::Url;
use vellum_contracts::{CredentialKind, Principal, Scope, ScopeSet};
use vellum_kernel::{Error, ErrorKind, Timestamp, UserId};

use crate::lockout::{Key, Lockouts};
use crate::oauth::{ClientMetadataFetcher, NoMetadataFetch, OAuthState};
use crate::password;
use crate::secret::{self, Kind, SecretHash};
use crate::sso::{Sso, SsoIdentity};
use crate::store::{ApiToken, Session, SessionMethod, SsoLink, State, Store, User, hash_key};

/// How often `last_seen_at` and `last_used_at` are written back, so a busy
/// session does not rewrite the store on every request.
const TOUCH_EVERY: Duration = Duration::from_mins(5);

/// The email of the built-in account break-glass sessions act as.
pub const BREAK_GLASS_EMAIL: &str = "break-glass";

/// The identity service's settings.
#[derive(Debug, Clone)]
pub struct IdentityConfig {
    /// The workspace's public base URL (`https://vellum.northstar.dev`),
    /// the `OAuth` issuer and the base of every token audience.
    pub public_url: Url,
    /// Whether cookies are `Secure` and use the `__Host-` prefix.
    pub https: bool,
    /// A session unused this long ends.
    pub session_idle: Duration,
    /// A session ends this long after sign-in, used or not.
    pub session_lifetime: Duration,
    /// How long a break-glass session lasts.
    pub break_glass_lifetime: Duration,
    /// How long an issued break-glass code stays redeemable.
    pub break_glass_code_lifetime: Duration,
    /// How long an `OAuth` access token lasts.
    pub access_token_lifetime: Duration,
    /// How long an `OAuth` refresh token lasts.
    pub refresh_token_lifetime: Duration,
    /// How long an `OAuth` authorization code lasts.
    pub code_lifetime: Duration,
    /// The data directory, where break-glass codes are dropped.
    pub data_dir: Option<PathBuf>,
}

impl IdentityConfig {
    /// Defaults for a workspace at `public_url`.
    #[must_use]
    pub fn new(public_url: Url) -> Self {
        let https = public_url.scheme() == "https";
        Self {
            public_url,
            https,
            session_idle: Duration::from_hours(336),
            session_lifetime: Duration::from_hours(720),
            break_glass_lifetime: Duration::from_mins(15),
            break_glass_code_lifetime: Duration::from_mins(15),
            access_token_lifetime: Duration::from_hours(1),
            refresh_token_lifetime: Duration::from_hours(720),
            code_lifetime: Duration::from_mins(1),
            data_dir: None,
        }
    }

    /// The session cookie's name: `__Host-vellum_session` over HTTPS, so
    /// the browser pins it to this host, path `/` and `Secure`.
    #[must_use]
    pub const fn cookie_name(&self) -> &'static str {
        if self.https {
            "__Host-vellum_session"
        } else {
            "vellum_session"
        }
    }

    /// The public base URL without a trailing slash.
    #[must_use]
    pub fn base(&self) -> String {
        self.public_url.as_str().trim_end_matches('/').to_owned()
    }
}

/// A freshly created session: the cookie value is shown only now.
#[derive(Debug, Clone)]
pub struct NewSession {
    /// The cookie value.
    pub secret: String,
    /// The stored record.
    pub session: Session,
}

/// A recognised credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The caller.
    pub principal: Principal,
    /// For cookie sessions, the value CSRF tokens bind to.
    pub csrf_binding: Option<SecretHash>,
}

/// What a valid bearer token grants, for resource servers such as MCP.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenInfo {
    /// A stable, non-secret ID of the token or grant.
    pub token_id: String,
    /// The user the token acts for.
    pub user: UserId,
    /// The user's display name.
    pub user_name: String,
    /// The `OAuth` client ID, or `api-token` for personal tokens.
    pub client_id: String,
    /// The client's display name, or the token's label.
    pub client_name: String,
    /// The effective scopes.
    pub scopes: ScopeSet,
    /// The resources the token is for; empty for personal tokens, which
    /// are valid everywhere.
    pub audience: Vec<String>,
    /// When it expires, if it does.
    pub expires_at: Option<Timestamp>,
    /// When the token (or its grant) was issued.
    pub issued_at: Timestamp,
}

/// A user as the API shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserView {
    /// The user's ID.
    pub id: UserId,
    /// The sign-in address.
    pub email: String,
    /// The display name.
    pub name: String,
    /// The unique handle used for assignees and mentions.
    pub handle: String,
    /// Whether the user administers the workspace.
    pub admin: bool,
    /// Whether sign-in is blocked.
    pub disabled: bool,
    /// Whether the account has a password (otherwise SSO only).
    pub has_password: bool,
    /// When the account was created.
    pub created_at: Timestamp,
}

impl From<&User> for UserView {
    fn from(user: &User) -> Self {
        Self {
            id: user.id,
            email: user.email.clone(),
            name: user.name.clone(),
            handle: user.handle.clone(),
            admin: user.admin,
            disabled: user.disabled,
            has_password: user.password_hash.is_some(),
            created_at: user.created_at,
        }
    }
}

/// Counts of people, from [`Identity::people`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PeopleCounts {
    /// Accounts, enabled or not.
    pub users: u64,
    /// Accounts that may sign in.
    pub enabled: u64,
    /// Enabled administrators.
    pub admins: u64,
    /// People seen in the last 24 hours.
    pub active_1d: u64,
    /// People seen in the last 7 days.
    pub active_7d: u64,
    /// People seen in the last 30 days.
    pub active_30d: u64,
    /// Browser sessions that have not expired.
    pub sessions: u64,
    /// Personal API tokens.
    pub api_tokens: u64,
}

/// The identity service.
pub struct Identity {
    config: IdentityConfig,
    store: Store,
    lockouts: Lockouts,
    hashing: password::Gate,
    pub(crate) oauth: OAuthState,
    pub(crate) fetcher: Arc<dyn ClientMetadataFetcher>,
    sso: Option<Arc<Sso>>,
}

impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

pub(crate) fn after(now: Timestamp, span: Duration) -> Timestamp {
    let ms = i64::try_from(span.as_millis()).unwrap_or(i64::MAX);
    Timestamp::from_unix_ms(now.unix_ms().saturating_add(ms)).unwrap_or(now)
}

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

fn elapsed(since: Timestamp, now: Timestamp) -> Duration {
    Duration::from_millis(u64::try_from(now.unix_ms() - since.unix_ms()).unwrap_or(0))
}

fn normalise_email(email: &str) -> Result<String, Error> {
    let email = email.trim().to_lowercase();
    let valid = email.len() <= 254
        && email
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && domain.contains('.'))
        && !email.chars().any(char::is_whitespace);
    if valid {
        Ok(email)
    } else {
        Err(Error::new(
            ErrorKind::InvalidInput,
            "enter a valid email address",
        ))
    }
}

fn check_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 120 {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "enter a name of 1 to 120 characters",
        ));
    }
    Ok(name.to_owned())
}

fn wrong_credentials() -> Error {
    Error::new(ErrorKind::Unauthenticated, "the email or password is wrong")
}

fn locked(retry_after: Duration) -> Error {
    Error::new(
        ErrorKind::RateLimited,
        format!(
            "too many failed attempts; try again in {} seconds",
            retry_after.as_secs().max(1)
        ),
    )
}

/// The scope ceiling of a user: everything but `admin` unless they are an
/// administrator.
#[must_use]
pub fn user_scopes(user: &User) -> ScopeSet {
    if user.admin {
        ScopeSet::FULL
    } else {
        ScopeSet::from_scopes([Scope::WriteDirect, Scope::Work])
    }
}

impl Identity {
    /// Creates the service over `store`.
    #[must_use]
    pub fn new(config: IdentityConfig, store: Store) -> Self {
        Self {
            config,
            store,
            lockouts: Lockouts::new(),
            hashing: password::Gate::default(),
            oauth: OAuthState::default(),
            fetcher: Arc::new(NoMetadataFetch),
            sso: None,
        }
    }

    /// Turns on single sign-on with one `OpenID Connect` provider.
    #[must_use]
    pub fn with_sso(mut self, sso: Sso) -> Self {
        self.sso = Some(Arc::new(sso));
        self
    }

    /// The single sign-on provider, when one is configured.
    #[must_use]
    pub const fn sso(&self) -> Option<&Arc<Sso>> {
        self.sso.as_ref()
    }

    /// Signs in the user a provider vouched for, and opens a session.
    ///
    /// The account is the one already linked to the provider's user ID.
    /// Failing that, a verified address links the account with that
    /// address, or, if the settings allow it, creates a member account
    /// without a password. Administrators are never created this way.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Conflict`] before set-up, [`ErrorKind::PermissionDenied`]
    /// for a disabled account or an address with no account, and
    /// [`ErrorKind::Unavailable`] when single sign-on is off.
    pub fn sso_sign_in(
        &self,
        found: &SsoIdentity,
        user_agent: Option<&str>,
    ) -> Result<NewSession, Error> {
        let sso = self.sso.as_ref().ok_or_else(|| {
            Error::new(ErrorKind::Unavailable, "single sign-on is not configured")
        })?;
        if self.needs_setup() {
            return Err(Error::new(
                ErrorKind::Conflict,
                "finish set-up first: the first administrator signs in with a password",
            ));
        }
        let link = SsoLink {
            issuer: found.issuer.clone(),
            subject: found.subject.clone(),
        };
        let email = found
            .email
            .as_deref()
            .filter(|_| found.email_verified)
            .map(normalise_email)
            .transpose()
            .ok()
            .flatten();
        let name = found
            .name
            .as_deref()
            .and_then(|n| check_name(n).ok())
            .or_else(|| {
                email
                    .as_deref()
                    .and_then(|e| e.split('@').next())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "Member".to_owned());
        let may_create = email.as_deref().is_some_and(|e| sso.config().may_create(e));
        let (user, created) = self.store.write(|s| {
            if let Some(user) = s.users.values().find(|u| u.sso.contains(&link)) {
                return Ok((user.clone(), false));
            }
            let Some(email) = email else {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    "the sign-in provider did not share a verified email address",
                ));
            };
            if email == BREAK_GLASS_EMAIL {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    "this account cannot use single sign-on",
                ));
            }
            if let Some(user) = s.users.values_mut().find(|u| u.email == email) {
                user.sso.push(link.clone());
                return Ok((user.clone(), false));
            }
            if !may_create {
                return Err(Error::new(
                    ErrorKind::PermissionDenied,
                    "there is no account for this address; ask an administrator to add you",
                ));
            }
            let user = User {
                id: UserId::generate(),
                handle: s.free_handle(&email),
                email,
                name: name.clone(),
                password_hash: None,
                admin: false,
                disabled: false,
                created_at: Timestamp::now(),
                sso: vec![link.clone()],
            };
            let _ = s.users.insert(user.id, user.clone());
            Ok((user, true))
        })??;
        if user.disabled {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "this account is disabled; ask an administrator",
            ));
        }
        tracing::info!(
            event = "identity.signed_in",
            user = %user.id,
            method = "sso",
            created,
            "signed in"
        );
        self.open_session(
            user.id,
            SessionMethod::Sso,
            self.config.session_lifetime,
            user_agent,
        )
    }

    /// Sets how Client ID Metadata Documents are fetched (through the SSRF
    /// guard in production).
    #[must_use]
    pub fn with_metadata_fetcher(mut self, fetcher: Arc<dyn ClientMetadataFetcher>) -> Self {
        self.fetcher = fetcher;
        self
    }

    /// The settings.
    #[must_use]
    pub const fn config(&self) -> &IdentityConfig {
        &self.config
    }

    /// The records.
    #[must_use]
    pub const fn store(&self) -> &Store {
        &self.store
    }

    /// Whether the workspace still needs its first administrator.
    #[must_use]
    pub fn needs_setup(&self) -> bool {
        self.store
            .read(|s| s.users.values().all(|u| u.email == BREAK_GLASS_EMAIL))
    }

    /// Creates the first administrator. Refused once any user exists.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Conflict`] after set-up, and the errors of
    /// [`Identity::create_user`].
    pub fn setup(&self, email: &str, name: &str, password: &str) -> Result<UserView, Error> {
        if !self.needs_setup() {
            return Err(Error::new(
                ErrorKind::Conflict,
                "this workspace is already set up",
            ));
        }
        self.create_user(email, name, Some(password), true)
    }

    /// Creates a user. `password` is `None` for SSO-only accounts.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] for a bad address, name or password and
    /// [`ErrorKind::Conflict`] when the address is taken.
    pub fn create_user(
        &self,
        email: &str,
        name: &str,
        password: Option<&str>,
        admin: bool,
    ) -> Result<UserView, Error> {
        let email = normalise_email(email)?;
        let name = check_name(name)?;
        let password_hash = match password {
            Some(p) => {
                password::check_policy(p)?;
                let _slot = self.hashing.permit()?;
                Some(password::hash(p)?)
            }
            None => None,
        };
        let mut user = User {
            id: UserId::generate(),
            email,
            name,
            handle: String::new(),
            password_hash,
            admin,
            disabled: false,
            created_at: Timestamp::now(),
            sso: Vec::new(),
        };
        self.store.write(|s| {
            if s.user_by_email(&user.email).is_some() {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "an account with this email already exists",
                ));
            }
            user.handle = s.free_handle(&user.email);
            let view = UserView::from(&user);
            let _ = s.users.insert(user.id, user);
            Ok(view)
        })?
    }

    /// Lists every user, oldest first.
    #[must_use]
    pub fn users(&self) -> Vec<UserView> {
        let mut users: Vec<UserView> = self.store.read(|s| {
            s.users
                .values()
                .filter(|u| u.email != BREAK_GLASS_EMAIL)
                .map(UserView::from)
                .collect()
        });
        users.sort_by_key(|u| u.created_at);
        users
    }

    /// How many people have accounts and how many used the server
    /// recently, for Admin → Usage and the usage metrics. The break-glass
    /// account is left out. A person counts as active in a window when one
    /// of their sessions was seen, or one of their API tokens used, in it.
    #[must_use]
    pub fn people(&self, now: Timestamp) -> PeopleCounts {
        self.store.read(|s| {
            let mut last: BTreeMap<UserId, i64> = BTreeMap::new();
            let seen = s
                .sessions
                .values()
                .map(|x| (x.user, x.last_seen_at.unix_ms()))
                .chain(
                    s.api_tokens
                        .values()
                        .filter_map(|t| t.last_used_at.map(|at| (t.user, at.unix_ms()))),
                );
            for (user, at) in seen {
                let entry = last.entry(user).or_insert(at);
                *entry = (*entry).max(at);
            }
            let mut counts = PeopleCounts {
                sessions: count(s.sessions.values().filter(|x| x.expires_at > now).count()),
                api_tokens: count(s.api_tokens.len()),
                ..PeopleCounts::default()
            };
            let day = 86_400_000_i64;
            for user in s.users.values().filter(|u| u.email != BREAK_GLASS_EMAIL) {
                counts.users += 1;
                counts.enabled += u64::from(!user.disabled);
                counts.admins += u64::from(user.admin && !user.disabled);
                if let Some(at) = last.get(&user.id) {
                    let ago = now.unix_ms().saturating_sub(*at);
                    counts.active_1d += u64::from(ago <= day);
                    counts.active_7d += u64::from(ago <= 7 * day);
                    counts.active_30d += u64::from(ago <= 30 * day);
                }
            }
            counts
        })
    }

    /// A user by ID.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown user.
    pub fn user(&self, id: UserId) -> Result<UserView, Error> {
        self.store
            .read(|s| s.users.get(&id).map(UserView::from))
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "user not found"))
    }

    /// Changes a user's name, administrator flag or disabled flag.
    /// Disabling revokes every credential the user holds. The last active
    /// administrator cannot lose their rights.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown user, and
    /// [`ErrorKind::Conflict`] when the change would leave no administrator.
    pub fn update_user(
        &self,
        id: UserId,
        name: Option<&str>,
        admin: Option<bool>,
        disabled: Option<bool>,
    ) -> Result<UserView, Error> {
        let name = name.map(check_name).transpose()?;
        self.store.write(|s| {
            let others_admin = s
                .users
                .values()
                .any(|u| u.id != id && u.admin && !u.disabled && u.email != BREAK_GLASS_EMAIL);
            let Some(user) = s.users.get_mut(&id) else {
                return Err(Error::new(ErrorKind::NotFound, "user not found"));
            };
            let demoted = admin == Some(false) || disabled == Some(true);
            if user.admin && demoted && !others_admin {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "the workspace needs at least one active administrator",
                ));
            }
            if let Some(name) = name {
                user.name = name;
            }
            if let Some(admin) = admin {
                user.admin = admin;
            }
            if let Some(disabled) = disabled {
                user.disabled = disabled;
            }
            let view = UserView::from(&*user);
            if view.disabled {
                s.revoke_user(id);
            }
            Ok(view)
        })?
    }

    /// Signs in with email and password and opens a session.
    ///
    /// Failed attempts lock out the address and the client IP with
    /// exponential back-off (after 5 and 20 free failures).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::RateLimited`] while locked out,
    /// [`ErrorKind::Unauthenticated`] for a wrong address or password, and
    /// [`ErrorKind::PermissionDenied`] for a disabled account.
    pub fn sign_in(
        &self,
        email: &str,
        password: &str,
        ip: Option<IpAddr>,
        user_agent: Option<&str>,
    ) -> Result<NewSession, Error> {
        let now = Instant::now();
        let email = email.trim().to_lowercase();
        if let Some(ip) = ip {
            self.lockouts
                .check(&self.store, Key::Ip(ip), now)
                .map_err(locked)?;
        }
        self.lockouts
            .check(&self.store, Key::Email(&email), now)
            .map_err(locked)?;
        // Refused while too many checks run, before any failure counts.
        let slot = self.hashing.permit()?;
        let user = self.store.read(|s| s.user_by_email(&email).cloned());
        let stored = user.as_ref().and_then(|u| u.password_hash.as_deref());
        let verified = stored.map_or_else(
            || {
                password::verify_dummy(password);
                false
            },
            |hash| password::verify(password, hash),
        );
        let Some(user) = user.filter(|_| verified) else {
            // The back-off starts now, after the slow password check, not
            // when the attempt began: otherwise a check slower than the
            // first wait (1 s) would let the next attempt straight through.
            let failed_at = Instant::now();
            self.lockouts
                .fail(&self.store, Key::Email(&email), failed_at);
            if let Some(ip) = ip {
                self.lockouts.fail(&self.store, Key::Ip(ip), failed_at);
            }
            tracing::info!(event = "identity.sign_in_failed", "sign-in failed");
            return Err(wrong_credentials());
        };
        self.lockouts.succeed(&self.store, Key::Email(&email));
        if user.disabled {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "this account is disabled; ask an administrator",
            ));
        }
        if user
            .password_hash
            .as_deref()
            .is_some_and(password::needs_rehash)
        {
            let upgraded = password::hash(password)?;
            drop(slot);
            self.store.write(|s| {
                if let Some(u) = s.users.get_mut(&user.id) {
                    u.password_hash = Some(upgraded);
                }
            })?;
        }
        tracing::info!(event = "identity.signed_in", user = %user.id, "signed in");
        self.open_session(
            user.id,
            SessionMethod::Password,
            self.config.session_lifetime,
            user_agent,
        )
    }

    /// Opens a session for a user who has just proved their identity
    /// (password, SSO or break-glass).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the random source or store fails.
    pub fn open_session(
        &self,
        user: UserId,
        method: SessionMethod,
        lifetime: Duration,
        user_agent: Option<&str>,
    ) -> Result<NewSession, Error> {
        let secret = secret::generate(Kind::Session)?;
        let hash = secret::hash(&secret);
        let now = Timestamp::now();
        let session = Session {
            id: secret::label(&hash),
            hash,
            user,
            method,
            created_at: now,
            last_seen_at: now,
            authenticated_at: now,
            expires_at: after(now, lifetime),
            user_agent: user_agent.map(|ua| ua.chars().take(256).collect()),
        };
        let stored = session.clone();
        self.store.write(|s| {
            s.sweep(now);
            let _ = s.sessions.insert(stored.id.clone(), stored);
        })?;
        Ok(NewSession { secret, session })
    }

    /// Resolves a session cookie. Unknown, expired, idle and malformed
    /// cookies resolve to `None`: the browser is simply signed out.
    #[must_use]
    pub fn session(&self, cookie: &str) -> Option<Resolved> {
        if !secret::well_formed(cookie, Kind::Session) {
            return None;
        }
        let hash = secret::hash(cookie);
        let now = Timestamp::now();
        let (session, user) = self.store.read(|s| {
            let session = s.session_by_hash(&hash)?.clone();
            let user = s.users.get(&session.user)?.clone();
            Some((session, user))
        })?;
        let idle = elapsed(session.last_seen_at, now) > self.config.session_idle;
        if session.expires_at <= now || idle || user.disabled {
            let _ = self.store.write(|s| s.sessions.remove(&session.id));
            return None;
        }
        if elapsed(session.last_seen_at, now) > TOUCH_EVERY {
            let _ = self.store.write(|s| {
                if let Some(stored) = s.sessions.get_mut(&session.id) {
                    stored.last_seen_at = now;
                }
            });
        }
        let kind = if session.method == SessionMethod::BreakGlass {
            CredentialKind::BreakGlass
        } else {
            CredentialKind::Session
        };
        Some(Resolved {
            principal: Principal::user(user.id, kind, user_scopes(&user), session.authenticated_at),
            csrf_binding: Some(session.hash),
        })
    }

    /// Ends the session behind a cookie. Unknown cookies are ignored.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the store cannot be written.
    pub fn sign_out(&self, cookie: &str) -> Result<(), Error> {
        let id = secret::label(&secret::hash(cookie));
        self.store.write(|s| {
            let _ = s.sessions.remove(&id);
        })
    }

    /// Proves the user's identity again, so admin actions that need a
    /// recent sign-in are allowed.
    ///
    /// # Errors
    ///
    /// As [`Identity::sign_in`], plus [`ErrorKind::Unauthenticated`] for an
    /// unknown session.
    pub fn reauthenticate(
        &self,
        cookie: &str,
        password: &str,
        ip: Option<IpAddr>,
    ) -> Result<(), Error> {
        let hash = secret::hash(cookie);
        let found = self.store.read(|s| {
            let session = s.session_by_hash(&hash)?;
            s.users
                .get(&session.user)
                .map(|u| (session.id.clone(), u.email.clone()))
        });
        let Some((session_id, email)) = found else {
            return Err(Error::new(
                ErrorKind::Unauthenticated,
                "sign in to continue",
            ));
        };
        // The same checks and lockout as signing in; the new session is
        // discarded and the existing one refreshed instead.
        let fresh = self.sign_in(&email, password, ip, None)?;
        self.store.write(|s| {
            let _ = s.sessions.remove(&fresh.session.id);
            if let Some(session) = s.sessions.get_mut(&session_id) {
                session.authenticated_at = Timestamp::now();
            }
        })
    }

    /// Changes a user's password after checking the current one, and ends
    /// every other session and grant.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for a wrong current password and
    /// [`ErrorKind::InvalidInput`] for a weak new one.
    pub fn change_password(
        &self,
        user: UserId,
        keep_session: Option<&str>,
        current: Option<&str>,
        new: &str,
    ) -> Result<(), Error> {
        password::check_policy(new)?;
        let stored = self
            .store
            .read(|s| s.users.get(&user).map(|u| u.password_hash.clone()))
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "user not found"))?;
        let _slot = self.hashing.permit()?;
        if let Some(hash) = stored {
            let ok = current.is_some_and(|c| password::verify(c, &hash));
            if !ok {
                return Err(Error::new(
                    ErrorKind::Unauthenticated,
                    "the current password is wrong",
                ));
            }
        }
        let new_hash = password::hash(new)?;
        let keep = keep_session.map(|c| secret::label(&secret::hash(c)));
        self.store.write(|s| {
            if let Some(u) = s.users.get_mut(&user) {
                u.password_hash = Some(new_hash);
            }
            s.sessions
                .retain(|id, session| session.user != user || keep.as_ref() == Some(id));
            s.grants.retain(|_, g| g.user != user);
            let grants = &s.grants;
            s.access_tokens.retain(|_, t| grants.contains_key(&t.grant));
        })
    }

    /// Lists a user's sessions, newest first.
    #[must_use]
    pub fn sessions(&self, user: UserId) -> Vec<Session> {
        let mut sessions: Vec<Session> = self.store.read(|s| {
            s.sessions
                .values()
                .filter(|x| x.user == user)
                .cloned()
                .collect()
        });
        sessions.sort_by_key(|s| std::cmp::Reverse(s.created_at));
        sessions
    }

    /// Ends one of a user's sessions.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] when the user has no such session.
    pub fn revoke_session(&self, user: UserId, id: &str) -> Result<(), Error> {
        self.store.write(|s| match s.sessions.get(id) {
            Some(session) if session.user == user => {
                let _ = s.sessions.remove(id);
                Ok(())
            }
            _ => Err(Error::new(ErrorKind::NotFound, "session not found")),
        })?
    }

    /// Creates a personal API token. The secret is returned only now.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] for an empty name or scope list, and
    /// [`ErrorKind::PermissionDenied`] for scopes beyond the user's own.
    pub fn create_api_token(
        &self,
        user: UserId,
        name: &str,
        scopes: &[Scope],
        expires_in_days: Option<u32>,
    ) -> Result<(String, ApiToken), Error> {
        let name = check_name(name)?;
        if scopes.is_empty() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "choose at least one scope",
            ));
        }
        let owner = self
            .store
            .read(|s| s.users.get(&user).cloned())
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "user not found"))?;
        let ceiling = user_scopes(&owner);
        if let Some(scope) = scopes.iter().find(|s| !ceiling.allows(**s)) {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                format!("you cannot grant the `{scope}` scope"),
            ));
        }
        let secret = secret::generate(Kind::ApiToken)?;
        let hash = secret::hash(&secret);
        let now = Timestamp::now();
        let token = ApiToken {
            id: secret::label(&hash),
            hash,
            user,
            name,
            scopes: scopes.to_vec(),
            created_at: now,
            expires_at: expires_in_days
                .map(|d| after(now, Duration::from_secs(u64::from(d) * 24 * 3600))),
            last_used_at: None,
        };
        let stored = token.clone();
        self.store.write(|s| {
            let _ = s.api_tokens.insert(stored.id.clone(), stored);
        })?;
        tracing::info!(event = "identity.api_token_created", user = %user, token = %token.id, "API token created");
        Ok((secret, token))
    }

    /// Lists a user's API tokens, newest first.
    #[must_use]
    pub fn api_tokens(&self, user: UserId) -> Vec<ApiToken> {
        let mut tokens: Vec<ApiToken> = self.store.read(|s| {
            s.api_tokens
                .values()
                .filter(|t| t.user == user)
                .cloned()
                .collect()
        });
        tokens.sort_by_key(|t| std::cmp::Reverse(t.created_at));
        tokens
    }

    /// Revokes one of a user's API tokens.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] when the user has no such token.
    pub fn revoke_api_token(&self, user: UserId, id: &str) -> Result<(), Error> {
        self.store.write(|s| match s.api_tokens.get(id) {
            Some(token) if token.user == user => {
                let _ = s.api_tokens.remove(id);
                Ok(())
            }
            _ => Err(Error::new(ErrorKind::NotFound, "token not found")),
        })?
    }

    /// Verifies a bearer token: a personal API token (`vlm_`) or an
    /// `OAuth` access token (`vat_`).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for an unknown, expired or revoked
    /// token, or one whose user is disabled.
    pub fn verify_token(&self, token: &str) -> Result<TokenInfo, Error> {
        let invalid = || Error::new(ErrorKind::Unauthenticated, "the access token is invalid");
        let now = Timestamp::now();
        let hash = secret::hash(token);
        match Kind::of(token) {
            Some(Kind::ApiToken) if secret::well_formed(token, Kind::ApiToken) => {
                let id = secret::label(&hash);
                let (record, user) = self
                    .store
                    .read(|s| {
                        let t = s.api_tokens.get(&id).filter(|t| t.hash == hash)?.clone();
                        let u = s.users.get(&t.user)?.clone();
                        Some((t, u))
                    })
                    .ok_or_else(invalid)?;
                if user.disabled || record.expires_at.is_some_and(|at| at <= now) {
                    return Err(invalid());
                }
                if record
                    .last_used_at
                    .is_none_or(|at| elapsed(at, now) > TOUCH_EVERY)
                {
                    let _ = self.store.write(|s| {
                        if let Some(t) = s.api_tokens.get_mut(&id) {
                            t.last_used_at = Some(now);
                        }
                    });
                }
                Ok(TokenInfo {
                    token_id: record.id,
                    user: user.id,
                    user_name: user.name.clone(),
                    client_id: "api-token".to_owned(),
                    client_name: record.name,
                    scopes: ScopeSet::from_scopes(record.scopes).intersect(user_scopes(&user)),
                    audience: Vec::new(),
                    expires_at: record.expires_at,
                    issued_at: record.created_at,
                })
            }
            Some(Kind::AccessToken) if secret::well_formed(token, Kind::AccessToken) => {
                let key = hash_key(&hash);
                self.store
                    .read(|s| {
                        let access = s.access_tokens.get(&key)?;
                        let grant = s.grants.get(&access.grant)?;
                        let user = s.users.get(&grant.user)?;
                        (access.expires_at > now && !user.disabled).then(|| TokenInfo {
                            token_id: grant.id.clone(),
                            user: user.id,
                            user_name: user.name.clone(),
                            client_id: grant.client_id.clone(),
                            client_name: grant.client_name.clone(),
                            scopes: ScopeSet::from_scopes(access.scopes.iter().copied())
                                .intersect(user_scopes(user)),
                            audience: access.audience.clone(),
                            expires_at: Some(access.expires_at),
                            issued_at: grant.auth_time.unwrap_or(grant.created_at),
                        })
                    })
                    .ok_or_else(invalid)
            }
            _ => Err(invalid()),
        }
    }

    /// Resolves a bearer token for the gateway. A personal token carries
    /// the time it was created (which took a recent sign-in) as its
    /// authentication time, and an `OAuth` token the sign-in time of the
    /// session that approved its grant (RT-039), so neither satisfies a
    /// recent-sign-in check for long.
    ///
    /// # Errors
    ///
    /// As [`Identity::verify_token`].
    pub fn bearer(&self, token: &str) -> Result<Resolved, Error> {
        let info = self.verify_token(token)?;
        Ok(Resolved {
            principal: Principal::user(
                info.user,
                CredentialKind::ApiToken,
                info.scopes,
                info.issued_at,
            ),
            csrf_binding: None,
        })
    }

    /// Writes a one-time break-glass code to
    /// `<data_dir>/.vellum/break-glass.json` and returns it. Only someone
    /// with access to the server's files can do this; redeeming the code
    /// opens a short administrator session even when SSO or every admin
    /// account is unavailable.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the file cannot be written.
    pub fn issue_break_glass(data_dir: &Path, lifetime: Duration) -> Result<String, Error> {
        let code = secret::generate(Kind::BreakGlass)?;
        let record = BreakGlassFile {
            hash: hash_key(&secret::hash(&code)),
            expires_at: after(Timestamp::now(), lifetime),
        };
        let bytes = serde_json::to_vec(&record)
            .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
        write_private(&break_glass_path(data_dir), &bytes)?;
        Ok(code)
    }

    /// Redeems a break-glass code for an administrator session. The code
    /// works once.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for a wrong, used or expired code;
    /// failures count towards the IP lockout.
    pub fn redeem_break_glass(
        &self,
        code: &str,
        ip: Option<IpAddr>,
        user_agent: Option<&str>,
    ) -> Result<NewSession, Error> {
        let now = Instant::now();
        if let Some(ip) = ip {
            self.lockouts
                .check(&self.store, Key::Ip(ip), now)
                .map_err(locked)?;
        }
        let refused = || {
            Error::new(
                ErrorKind::Unauthenticated,
                "the break-glass code is not valid",
            )
        };
        let Some(dir) = self.config.data_dir.as_deref() else {
            return Err(refused());
        };
        let path = break_glass_path(dir);
        let record: Option<BreakGlassFile> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok());
        let presented = hash_key(&secret::hash(code));
        let valid = record.as_ref().is_some_and(|r| {
            bool::from(r.hash.as_bytes().ct_eq(presented.as_bytes()))
                && r.expires_at > Timestamp::now()
        });
        if !valid {
            if let Some(ip) = ip {
                self.lockouts.fail(&self.store, Key::Ip(ip), now);
            }
            tracing::warn!(
                event = "identity.break_glass_refused",
                "break-glass code refused"
            );
            return Err(refused());
        }
        std::fs::remove_file(&path)
            .map_err(|e| Error::new(ErrorKind::Internal, format!("cannot spend the code: {e}")))?;
        let user = self.break_glass_user()?;
        tracing::warn!(
            event = "identity.break_glass_used",
            "break-glass administrator session opened"
        );
        self.open_session(
            user,
            SessionMethod::BreakGlass,
            self.config.break_glass_lifetime,
            user_agent,
        )
    }

    fn break_glass_user(&self) -> Result<UserId, Error> {
        self.store.write(|s| {
            if let Some(u) = s.user_by_email(BREAK_GLASS_EMAIL) {
                return u.id;
            }
            let user = User {
                id: UserId::generate(),
                email: BREAK_GLASS_EMAIL.to_owned(),
                name: "Break-glass administrator".to_owned(),
                handle: s.free_handle("break-glass@local"),
                password_hash: None,
                admin: true,
                disabled: false,
                created_at: Timestamp::now(),
                sso: Vec::new(),
            };
            let id = user.id;
            let _ = s.users.insert(id, user);
            id
        })
    }

    /// The user behind a handle, for services that store handles.
    #[must_use]
    pub fn user_by_handle(&self, handle: &str) -> Option<UserView> {
        self.store
            .read(|s| s.user_by_handle(handle).map(UserView::from))
    }

    /// Reads the records for a closure (used by the `OAuth` server).
    pub(crate) fn with_state<R>(&self, f: impl FnOnce(&State) -> R) -> R {
        self.store.read(f)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct BreakGlassFile {
    hash: String,
    expires_at: Timestamp,
}

fn break_glass_path(data_dir: &Path) -> PathBuf {
    data_dir.join(".vellum").join("break-glass.json")
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    use std::io::Write as _;
    let fail = |e: std::io::Error| {
        Error::new(
            ErrorKind::Internal,
            format!("cannot write {}: {e}", path.display()),
        )
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let mut options = std::fs::OpenOptions::new();
    let _ = options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let _ = options.mode(0o600);
    }
    let mut file = options.open(path).map_err(fail)?;
    file.write_all(bytes).map_err(fail)
}
