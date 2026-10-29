// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Who is calling: the authenticated principal and its token scopes.
//!
//! The gateway resolves every request's credential (session cookie, API
//! token, break-glass session or share link) into a [`Principal`] before a
//! service sees it. Scopes are a coarse ceiling on what a credential may do;
//! per-object access is still decided by the authorization service (deny by
//! default).

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_kernel::{Error, ErrorKind, ShareId, Timestamp, UserId};

/// A permission ceiling carried by a credential.
///
/// The set matches the MCP scopes in the brief: `read`, `write:draft`,
/// `write:direct`, `work` and `admin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Read anything the user can see.
    #[serde(rename = "read")]
    Read,
    /// Create drafts and propose edits for human review.
    #[serde(rename = "write:draft")]
    WriteDraft,
    /// Write directly, without a draft.
    #[serde(rename = "write:direct")]
    WriteDirect,
    /// Issue tracking and My Work.
    #[serde(rename = "work")]
    Work,
    /// Administration. Admin operations also need a recent sign-in.
    #[serde(rename = "admin")]
    Admin,
}

impl Scope {
    /// Every scope, lowest first.
    pub const ALL: [Self; 5] = [
        Self::Read,
        Self::WriteDraft,
        Self::WriteDirect,
        Self::Work,
        Self::Admin,
    ];

    /// The scope's wire name, as used in tokens and the OpenAPI document.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::WriteDraft => "write:draft",
            Self::WriteDirect => "write:direct",
            Self::Work => "work",
            Self::Admin => "admin",
        }
    }

    const fn bit(self) -> u8 {
        match self {
            Self::Read => 1,
            Self::WriteDraft => 1 << 1,
            Self::WriteDirect => 1 << 2,
            Self::Work => 1 << 3,
            Self::Admin => 1 << 4,
        }
    }

    /// The scopes this one implies, itself included.
    ///
    /// `admin` implies every scope, `write:direct` implies `write:draft`,
    /// and every writing scope implies `read`.
    #[must_use]
    pub const fn closure(self) -> ScopeSet {
        let bits = match self {
            Self::Read => Self::Read.bit(),
            Self::WriteDraft => Self::WriteDraft.bit() | Self::Read.bit(),
            Self::WriteDirect => {
                Self::WriteDirect.bit() | Self::WriteDraft.bit() | Self::Read.bit()
            }
            Self::Work => Self::Work.bit() | Self::Read.bit(),
            Self::Admin => ScopeSet::FULL.0,
        };
        ScopeSet(bits)
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Scope {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|scope| scope.as_str() == text)
            .ok_or_else(|| Error::new(ErrorKind::InvalidInput, format!("unknown scope `{text}`")))
    }
}

impl Serialize for Scope {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Scope {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// A set of scopes, closed under implication.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ScopeSet(u8);

impl ScopeSet {
    /// No scopes: the anonymous caller.
    pub const EMPTY: Self = Self(0);
    /// Every scope: a signed-in administrator's session.
    pub const FULL: Self = Self(0b1_1111);

    /// Builds a set from scopes, adding everything they imply.
    #[must_use]
    pub fn from_scopes(scopes: impl IntoIterator<Item = Scope>) -> Self {
        scopes
            .into_iter()
            .fold(Self::EMPTY, |set, scope| Self(set.0 | scope.closure().0))
    }

    /// Whether the set allows `scope`.
    #[must_use]
    pub const fn allows(self, scope: Scope) -> bool {
        self.0 & scope.bit() != 0
    }

    /// The scopes in both sets.
    #[must_use]
    pub const fn intersect(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Whether the set holds no scope.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The scopes in the set, lowest first.
    pub fn iter(self) -> impl Iterator<Item = Scope> {
        Scope::ALL.into_iter().filter(move |s| self.allows(*s))
    }
}

impl fmt::Debug for ScopeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries(self.iter().map(Scope::as_str))
            .finish()
    }
}

impl FromIterator<Scope> for ScopeSet {
    fn from_iter<T: IntoIterator<Item = Scope>>(iter: T) -> Self {
        Self::from_scopes(iter)
    }
}

/// How the caller proved who they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// No credential.
    Anonymous,
    /// A browser session cookie.
    Session,
    /// A scoped API token (CLI, integrations, MCP over HTTP).
    ApiToken,
    /// A short-lived break-glass administrator session.
    BreakGlass,
    /// A public share link; reaches only the shared object.
    ShareLink,
}

/// The authenticated caller of a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    user: Option<UserId>,
    kind: CredentialKind,
    scopes: ScopeSet,
    authenticated_at: Option<Timestamp>,
    share: Option<ShareId>,
}

impl Principal {
    /// The caller with no credential. It holds no scope.
    #[must_use]
    pub const fn anonymous() -> Self {
        Self {
            user: None,
            kind: CredentialKind::Anonymous,
            scopes: ScopeSet::EMPTY,
            authenticated_at: None,
            share: None,
        }
    }

    /// A signed-in user. `authenticated_at` is when they last proved their
    /// identity (password, SSO or re-authentication), not when the session
    /// was last used.
    #[must_use]
    pub const fn user(
        user: UserId,
        kind: CredentialKind,
        scopes: ScopeSet,
        authenticated_at: Timestamp,
    ) -> Self {
        Self {
            user: Some(user),
            kind,
            scopes,
            authenticated_at: Some(authenticated_at),
            share: None,
        }
    }

    /// A visitor holding a share link. It may read only the shared object.
    #[must_use]
    pub const fn share_link(share: ShareId) -> Self {
        Self {
            user: None,
            kind: CredentialKind::ShareLink,
            scopes: ScopeSet::from_bits(Scope::Read.bit()),
            authenticated_at: None,
            share: Some(share),
        }
    }

    /// The user, when one is signed in.
    #[must_use]
    pub const fn user_id(&self) -> Option<UserId> {
        self.user
    }

    /// How the caller authenticated.
    #[must_use]
    pub const fn kind(&self) -> CredentialKind {
        self.kind
    }

    /// The credential's scope ceiling.
    #[must_use]
    pub const fn scopes(&self) -> ScopeSet {
        self.scopes
    }

    /// The share link, for a share-link visitor.
    #[must_use]
    pub const fn share_id(&self) -> Option<ShareId> {
        self.share
    }

    /// When the caller last proved their identity.
    #[must_use]
    pub const fn authenticated_at(&self) -> Option<Timestamp> {
        self.authenticated_at
    }

    /// Returns the signed-in user, or an `unauthenticated` error.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] when no user is signed in.
    pub fn require_user(&self) -> Result<UserId, Error> {
        self.user
            .ok_or_else(|| Error::new(ErrorKind::Unauthenticated, "sign in to continue"))
    }

    /// Checks that the credential carries `scope`.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for an anonymous caller, and
    /// [`ErrorKind::PermissionDenied`] when the scope is missing.
    pub fn require_scope(&self, scope: Scope) -> Result<(), Error> {
        if self.scopes.allows(scope) {
            return Ok(());
        }
        if self.kind == CredentialKind::Anonymous {
            return Err(Error::new(
                ErrorKind::Unauthenticated,
                "sign in to continue",
            ));
        }
        Err(Error::new(
            ErrorKind::PermissionDenied,
            format!("this credential lacks the `{scope}` scope"),
        ))
    }

    /// Checks that the caller proved their identity within `max_age` of
    /// `now`. Admin actions call this so a stolen idle session cannot
    /// perform them.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] with the message `reauthentication
    /// required`, which clients answer by prompting for the password or SSO.
    pub fn require_fresh(&self, max_age: Duration, now: Timestamp) -> Result<(), Error> {
        let limit = i64::try_from(max_age.as_millis()).unwrap_or(i64::MAX);
        let fresh = self
            .authenticated_at
            .is_some_and(|at| now.unix_ms().saturating_sub(at.unix_ms()) <= limit);
        if fresh {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::Unauthenticated,
                REAUTHENTICATION_REQUIRED,
            ))
        }
    }
}

/// The message of the error asking the client to re-authenticate.
pub const REAUTHENTICATION_REQUIRED: &str = "reauthentication required";

impl ScopeSet {
    const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implication() {
        let set = ScopeSet::from_scopes([Scope::WriteDirect]);
        assert!(set.allows(Scope::WriteDraft));
        assert!(set.allows(Scope::Read));
        assert!(!set.allows(Scope::Work));
        assert!(!set.allows(Scope::Admin));
        assert_eq!(ScopeSet::from_scopes([Scope::Admin]), ScopeSet::FULL);
        assert_eq!(
            ScopeSet::FULL.intersect(Scope::Work.closure()),
            ScopeSet::from_scopes([Scope::Work])
        );
    }

    #[test]
    fn wire_names_round_trip() {
        for scope in Scope::ALL {
            assert_eq!(scope.as_str().parse::<Scope>().ok(), Some(scope));
            let json = serde_json::to_string(&scope).ok();
            assert_eq!(json, Some(format!("\"{scope}\"")));
        }
        assert!("root".parse::<Scope>().is_err());
    }

    #[test]
    fn anonymous_is_unauthenticated_and_users_are_denied() {
        let anon = Principal::anonymous();
        assert_eq!(
            anon.require_scope(Scope::Read).err().map(|e| e.kind()),
            Some(ErrorKind::Unauthenticated)
        );
        let user = Principal::user(
            UserId::generate(),
            CredentialKind::ApiToken,
            ScopeSet::from_scopes([Scope::Read]),
            Timestamp::now(),
        );
        assert!(user.require_scope(Scope::Read).is_ok());
        assert_eq!(
            user.require_scope(Scope::WriteDraft)
                .err()
                .map(|e| e.kind()),
            Some(ErrorKind::PermissionDenied)
        );
    }

    #[test]
    fn freshness() -> Result<(), Box<dyn std::error::Error>> {
        let at = Timestamp::from_unix_ms(1_000_000)?;
        let user = Principal::user(
            UserId::generate(),
            CredentialKind::Session,
            ScopeSet::FULL,
            at,
        );
        let window = Duration::from_mins(5);
        assert!(
            user.require_fresh(window, Timestamp::from_unix_ms(1_299_999)?)
                .is_ok()
        );
        let stale = user.require_fresh(window, Timestamp::from_unix_ms(1_300_001)?);
        assert_eq!(
            stale.err().map(|e| e.message().to_owned()).as_deref(),
            Some(REAUTHENTICATION_REQUIRED)
        );
        assert!(Principal::anonymous().require_fresh(window, at).is_err());
        Ok(())
    }

    #[test]
    fn share_links_read_only() {
        let p = Principal::share_link(ShareId::generate());
        assert!(p.require_scope(Scope::Read).is_ok());
        assert!(p.require_scope(Scope::WriteDraft).is_err());
        assert!(p.require_user().is_err());
    }
}
