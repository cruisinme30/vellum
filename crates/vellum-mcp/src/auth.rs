// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Scopes, bearer tokens and the `OAuth` 2.1 resource-server role.
//!
//! The MCP endpoint is an `OAuth` 2.1 *resource server*. The identity
//! service is the authorization server (Client ID Metadata Documents,
//! PKCE, resource indicators); this module only verifies the tokens it
//! issues through the [`TokenVerifier`] port, checks that each token was
//! issued for this endpoint (RFC 8707 audience), publishes the Protected
//! Resource Metadata document (RFC 9728) and builds the `WWW-Authenticate`
//! challenges (RFC 6750).

use std::collections::BTreeSet;
use std::fmt;
use std::time::SystemTime;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A permission carried by a token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Scope {
    /// Read and search everything the user can see.
    #[serde(rename = "read")]
    Read,
    /// Write pages and rows as drafts for human review.
    #[serde(rename = "write:draft")]
    WriteDraft,
    /// Write pages and rows directly, without review.
    #[serde(rename = "write:direct")]
    WriteDirect,
    /// File and update issues, comments and tasks.
    #[serde(rename = "work")]
    Work,
    /// Administrative operations.
    #[serde(rename = "admin")]
    Admin,
}

impl Scope {
    /// Every scope, in canonical order.
    pub const ALL: [Self; 5] = [
        Self::Read,
        Self::WriteDraft,
        Self::WriteDirect,
        Self::Work,
        Self::Admin,
    ];

    /// The wire name.
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

    /// Parses a wire name.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.as_str() == name)
    }

    /// The scopes this one implies, itself included (decision D85).
    #[must_use]
    pub const fn implied(self) -> &'static [Self] {
        match self {
            Self::Read => &[Self::Read],
            Self::WriteDraft => &[Self::WriteDraft, Self::Read],
            Self::WriteDirect => &[Self::WriteDirect, Self::WriteDraft, Self::Read],
            Self::Work => &[Self::Work, Self::Read],
            Self::Admin => &Self::ALL,
        }
    }
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The effective scopes of a token, with the hierarchy expanded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeSet(BTreeSet<Scope>);

impl ScopeSet {
    /// Builds a set from granted scopes, expanding implications.
    pub fn new(granted: impl IntoIterator<Item = Scope>) -> Self {
        Self(
            granted
                .into_iter()
                .flat_map(|s| s.implied().iter().copied())
                .collect(),
        )
    }

    /// Parses a space-separated `OAuth` `scope` string; unknown names are
    /// ignored.
    #[must_use]
    pub fn parse(scope: &str) -> Self {
        Self::new(scope.split_whitespace().filter_map(Scope::parse))
    }

    /// Whether the set grants `scope`.
    #[must_use]
    pub fn allows(&self, scope: Scope) -> bool {
        self.0.contains(&scope)
    }

    /// The scopes in canonical order.
    pub fn iter(&self) -> impl Iterator<Item = Scope> + '_ {
        self.0.iter().copied()
    }
}

impl fmt::Display for ScopeSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.iter().map(Scope::as_str).collect();
        f.write_str(&names.join(" "))
    }
}

/// What the identity service knows about a valid token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedToken {
    /// Stable id of the token (never the secret), for audit and rate limits.
    pub token_id: String,
    /// The user the client acts for.
    pub user_id: String,
    /// The user's display name, for attribution in drafts.
    pub user_name: String,
    /// The `OAuth` client id (a Client ID Metadata Document URL, or an API
    /// token label).
    pub client_id: String,
    /// The client's display name.
    pub client_name: String,
    /// The granted scopes.
    pub scopes: ScopeSet,
    /// Audiences the token was issued for (RFC 8707 resource values).
    pub audience: Vec<String>,
    /// When the token expires, if it does.
    pub expires_at: Option<SystemTime>,
}

/// Why a token was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// Unknown, revoked or malformed token.
    #[error("the access token is invalid")]
    Invalid,
    /// The token has expired.
    #[error("the access token has expired")]
    Expired,
    /// The identity service could not be reached.
    #[error("the token could not be verified: {0}")]
    Unavailable(String),
}

/// Port: verifies bearer tokens. Implemented by the identity service.
#[async_trait]
pub trait TokenVerifier: Send + Sync {
    /// Looks up a bearer token.
    ///
    /// # Errors
    ///
    /// [`TokenError`] when the token is not valid.
    async fn verify(&self, token: &str) -> Result<VerifiedToken, TokenError>;
}

/// The authenticated caller of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    /// The token id.
    pub token_id: String,
    /// The user id.
    pub user_id: String,
    /// The user's display name.
    pub user_name: String,
    /// The client id.
    pub client_id: String,
    /// The client's display name.
    pub client_name: String,
    /// Effective scopes.
    pub scopes: ScopeSet,
}

impl Principal {
    /// Builds a principal from a token after the audience check.
    #[must_use]
    pub fn from_token(token: VerifiedToken) -> Self {
        Self {
            token_id: token.token_id,
            user_id: token.user_id,
            user_name: token.user_name,
            client_id: token.client_id,
            client_name: token.client_name,
            scopes: token.scopes,
        }
    }

    /// Whether the principal holds `scope`.
    #[must_use]
    pub fn allows(&self, scope: Scope) -> bool {
        self.scopes.allows(scope)
    }
}

/// Checks a verified token against expiry and this endpoint's audience.
///
/// # Errors
///
/// [`TokenError::Expired`] or [`TokenError::Invalid`] (wrong audience).
pub fn admit(
    token: VerifiedToken,
    resource: &str,
    now: SystemTime,
) -> Result<Principal, TokenError> {
    if token.expires_at.is_some_and(|t| t <= now) {
        return Err(TokenError::Expired);
    }
    let canonical = canonical_resource(resource);
    if !token
        .audience
        .iter()
        .any(|a| canonical_resource(a) == canonical)
    {
        return Err(TokenError::Invalid);
    }
    Ok(Principal::from_token(token))
}

/// Canonical form of a resource URI: lowercase scheme and host, no
/// trailing slash, no fragment.
#[must_use]
pub fn canonical_resource(uri: &str) -> String {
    let uri = uri.split('#').next().unwrap_or_default();
    let (scheme, rest) = uri.split_once("://").unwrap_or(("", uri));
    let (authority, path) = rest.find('/').map_or((rest, ""), |i| rest.split_at(i));
    let path = path.trim_end_matches('/');
    format!(
        "{}://{}{}",
        scheme.to_ascii_lowercase(),
        authority.to_ascii_lowercase(),
        path
    )
}

/// Where this resource server lives and who issues its tokens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceConfig {
    /// Public base URL of the deployment, such as `https://vellum.northstar.dev`.
    pub base_url: String,
    /// Issuer identifier of the authorization server (the identity service).
    pub issuer: String,
}

impl ResourceConfig {
    /// The canonical URI of the MCP endpoint, the RFC 8707 resource.
    #[must_use]
    pub fn resource(&self) -> String {
        canonical_resource(&format!("{}/mcp", self.base_url.trim_end_matches('/')))
    }

    /// The URL of the Protected Resource Metadata document.
    #[must_use]
    pub fn metadata_url(&self) -> String {
        format!(
            "{}/.well-known/oauth-protected-resource/mcp",
            self.base_url.trim_end_matches('/')
        )
    }

    /// The RFC 9728 Protected Resource Metadata document.
    ///
    /// `scopes_supported` lists the minimal scopes for everyday use;
    /// `write:direct` and `admin` are asked for by step-up challenges.
    #[must_use]
    pub fn metadata(&self) -> Value {
        json!({
            "resource": self.resource(),
            "authorization_servers": [self.issuer],
            "scopes_supported": ["read", "write:draft", "work"],
            "bearer_methods_supported": ["header"],
            "resource_name": "Vellum",
            "resource_documentation": format!("{}/help/ai-and-mcp", self.base_url.trim_end_matches('/')),
        })
    }

    /// `WWW-Authenticate` for a missing or invalid token (HTTP 401).
    #[must_use]
    pub fn unauthorized_challenge(&self, error: Option<&TokenError>) -> String {
        let mut parts = vec![format!("resource_metadata=\"{}\"", self.metadata_url())];
        parts.push("scope=\"read write:draft work\"".to_owned());
        if let Some(e) = error {
            parts.push("error=\"invalid_token\"".to_owned());
            parts.push(format!("error_description=\"{e}\""));
        }
        format!("Bearer {}", parts.join(", "))
    }

    /// `WWW-Authenticate` for a token that lacks scopes (HTTP 403).
    #[must_use]
    pub fn insufficient_scope_challenge(&self, needed: &[Scope]) -> String {
        let names: Vec<&str> = needed.iter().map(|s| s.as_str()).collect();
        format!(
            "Bearer error=\"insufficient_scope\", scope=\"{}\", resource_metadata=\"{}\", error_description=\"This operation needs the {} scope\"",
            names.join(" "),
            self.metadata_url(),
            names.join(" and ")
        )
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn token(aud: &str) -> VerifiedToken {
        VerifiedToken {
            token_id: "tok_1".into(),
            user_id: "usr_priya".into(),
            user_name: "Priya".into(),
            client_id: "https://claude.ai/oauth/client.json".into(),
            client_name: "Claude".into(),
            scopes: ScopeSet::parse("read"),
            audience: vec![aud.into()],
            expires_at: None,
        }
    }

    #[test]
    fn scopes_expand_their_hierarchy() {
        let set = ScopeSet::parse("write:direct bogus");
        assert!(set.allows(Scope::Read));
        assert!(set.allows(Scope::WriteDraft));
        assert!(!set.allows(Scope::Work));
        assert!(ScopeSet::parse("admin").allows(Scope::WriteDirect));
        assert_eq!(ScopeSet::parse("work").to_string(), "read work");
    }

    #[test]
    fn audience_must_match_the_endpoint() {
        let now = SystemTime::now();
        let resource = "https://vellum.northstar.dev/mcp";
        assert!(admit(token("HTTPS://Vellum.northstar.dev/mcp/"), resource, now).is_ok());
        assert_eq!(
            admit(token("https://other.example/mcp"), resource, now).err(),
            Some(TokenError::Invalid)
        );
        let mut expired = token(resource);
        expired.expires_at = now.checked_sub(Duration::from_secs(1));
        assert_eq!(
            admit(expired, resource, now).err(),
            Some(TokenError::Expired)
        );
    }

    #[test]
    fn metadata_and_challenges_point_at_each_other() {
        let cfg = ResourceConfig {
            base_url: "https://vellum.northstar.dev/".into(),
            issuer: "https://vellum.northstar.dev".into(),
        };
        assert_eq!(
            cfg.metadata()["resource"],
            "https://vellum.northstar.dev/mcp"
        );
        assert!(
            cfg.unauthorized_challenge(None)
                .contains(&cfg.metadata_url())
        );
        let c = cfg.insufficient_scope_challenge(&[Scope::WriteDirect]);
        assert!(c.contains("error=\"insufficient_scope\""));
        assert!(c.contains("scope=\"write:direct\""));
    }
}
