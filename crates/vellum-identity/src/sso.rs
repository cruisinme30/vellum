// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Single sign-on with an `OpenID Connect` provider (Okta, Entra ID,
//! Google Workspace, Keycloak, Authentik...).
//!
//! Vellum is a relying party using the authorization code flow with PKCE:
//!
//! 1. [`Sso::start`] reads the provider's discovery document, remembers a
//!    random `state`, `nonce` and PKCE verifier for ten minutes, and
//!    returns the provider's sign-in URL. The browser also gets the
//!    `state` in a short-lived cookie, so a callback started in another
//!    browser (login CSRF) is refused.
//! 2. [`Sso::finish`] exchanges the code for tokens and verifies the ID
//!    token: an `RS256` or `ES256` signature from the provider's key set,
//!    the issuer, the audience, the expiry and the nonce.
//!
//! What happens next (linking to an account and opening a session) is
//! [`crate::Identity::sso_sign_in`]. The resulting session is an ordinary
//! Vellum session: it keeps working while the provider is down.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED, RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents, UnparsedPublicKey,
};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;
use url::Url;
use vellum_contracts::BoxFuture;
use vellum_kernel::{Error, ErrorKind, Timestamp};

/// How long a sign-in may take at the provider.
pub const PENDING_LIFETIME: Duration = Duration::from_mins(10);

/// Sign-ins in flight at once; more are refused until some finish.
const MAX_PENDING: usize = 10_000;

/// How long the discovery document and key set are trusted.
const METADATA_LIFETIME: Duration = Duration::from_hours(1);

/// The least time between two key set downloads triggered by an unknown
/// key ID, so forged tokens cannot make Vellum hammer the provider.
const JWKS_REFRESH_GAP: Duration = Duration::from_secs(10);

/// Clock difference tolerated between Vellum and the provider.
const CLOCK_SKEW_SECS: i64 = 60;

/// The provider and how its users map to Vellum accounts.
#[derive(Clone)]
pub struct SsoConfig {
    /// The issuer URL, exactly as the provider's tokens name it.
    pub issuer: Url,
    /// The client ID registered with the provider.
    pub client_id: String,
    /// The client secret; `None` for a public client (PKCE only).
    pub client_secret: Option<String>,
    /// The button label: "Sign in with {name}".
    pub display_name: String,
    /// Email domains whose users get an account on first sign-in. Empty
    /// means any domain, when [`SsoConfig::auto_create`] is on.
    pub allowed_domains: Vec<String>,
    /// Whether a verified address with no account gets one.
    pub auto_create: bool,
}

impl fmt::Debug for SsoConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SsoConfig")
            .field("issuer", &self.issuer.as_str())
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "***"))
            .field("display_name", &self.display_name)
            .field("allowed_domains", &self.allowed_domains)
            .field("auto_create", &self.auto_create)
            .finish()
    }
}

impl SsoConfig {
    /// Whether an account may be created for `email`.
    #[must_use]
    pub fn may_create(&self, email: &str) -> bool {
        self.auto_create && self.may_link(email)
    }

    /// Whether this provider may sign in to an existing account with
    /// `email`: any address without a domain list, otherwise only one in
    /// the list (RT-043, D315).
    #[must_use]
    pub fn may_link(&self, email: &str) -> bool {
        in_domains(&self.allowed_domains, email)
    }
}

/// Whether `email` is in one of `domains`; an empty list allows any.
pub(crate) fn in_domains(domains: &[String], email: &str) -> bool {
    let domain = email.rsplit_once('@').map_or("", |(_, d)| d);
    domains.is_empty()
        || domains
            .iter()
            .any(|d| d.trim_start_matches('@').eq_ignore_ascii_case(domain))
}

/// The HTTP calls single sign-on makes, through the SSRF guard in
/// production.
pub trait SsoHttp: Send + Sync + 'static {
    /// GETs `url` and returns the body of a 2xx response.
    fn get<'a>(&'a self, url: &'a str) -> BoxFuture<'a, Result<Vec<u8>, Error>>;

    /// POSTs `form` (URL-encoded) to `url`, with HTTP basic credentials
    /// when given, and returns the status and body.
    fn post_form<'a>(
        &'a self,
        url: &'a str,
        form: Vec<(String, String)>,
        basic: Option<(String, String)>,
    ) -> BoxFuture<'a, Result<(u16, Vec<u8>), Error>>;
}

/// What a verified ID token says about the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsoIdentity {
    /// The provider's issuer URL.
    pub issuer: String,
    /// The provider's stable user ID.
    pub subject: String,
    /// The address, lower-cased, if the provider shared one.
    pub email: Option<String>,
    /// Whether the provider verified the address.
    pub email_verified: bool,
    /// The display name, if shared.
    pub name: Option<String>,
}

/// A started sign-in: send the browser to `url`, and set the state cookie.
#[derive(Debug, Clone)]
pub struct SsoStart {
    /// The provider's sign-in URL.
    pub url: String,
    /// The value for the state cookie.
    pub state: String,
}

/// A finished sign-in.
#[derive(Debug, Clone)]
pub struct SsoFinish {
    /// Who signed in.
    pub identity: SsoIdentity,
    /// Where to go afterwards (a local path).
    pub next: String,
}

#[derive(Debug)]
struct Pending {
    nonce: String,
    verifier: String,
    next: String,
    reauth: bool,
    started: Instant,
}

#[derive(Debug, Clone, Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    kty: String,
    #[serde(default)]
    kid: Option<String>,
    #[serde(default, rename = "use")]
    usage: Option<String>,
    #[serde(default)]
    alg: Option<String>,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
    #[serde(default)]
    crv: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JwkSet {
    keys: Vec<Jwk>,
}

#[derive(Debug, Default)]
struct Cache {
    discovery: Option<(Discovery, Instant)>,
    keys: Option<(Vec<Jwk>, Instant)>,
}

/// The relying party.
pub struct Sso {
    config: SsoConfig,
    http: Arc<dyn SsoHttp>,
    pending: Mutex<HashMap<String, Pending>>,
    cache: Mutex<Cache>,
}

impl fmt::Debug for Sso {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sso")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

fn failed(message: impl Into<std::borrow::Cow<'static, str>>) -> Error {
    Error::new(ErrorKind::Unauthenticated, message)
}

fn unavailable(message: impl fmt::Display) -> Error {
    Error::new(
        ErrorKind::Unavailable,
        format!("the sign-in provider cannot be reached: {message}"),
    )
}

fn random() -> Result<String, Error> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| Error::new(ErrorKind::Internal, format!("random source failed: {e}")))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// A local path to return to: absolute, and not another origin (`//x`,
/// `/\x`). Anything else becomes `/`.
#[must_use]
pub fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n)
            if n.starts_with('/')
                && !n.starts_with("//")
                && !n.starts_with("/\\")
                && !n.chars().any(char::is_control) =>
        {
            n.to_owned()
        }
        _ => "/".to_owned(),
    }
}

impl Sso {
    /// The relying party for `config`, calling out through `http`.
    #[must_use]
    pub fn new(config: SsoConfig, http: Arc<dyn SsoHttp>) -> Self {
        Self {
            config,
            http,
            pending: Mutex::new(HashMap::new()),
            cache: Mutex::new(Cache::default()),
        }
    }

    /// The settings.
    #[must_use]
    pub const fn config(&self) -> &SsoConfig {
        &self.config
    }

    async fn get_json<T: for<'de> Deserialize<'de>>(&self, url: &str) -> Result<T, Error> {
        let body = self.http.get(url).await.map_err(unavailable)?;
        serde_json::from_slice(&body)
            .map_err(|e| unavailable(format!("unreadable response from {url}: {e}")))
    }

    async fn discovery(&self) -> Result<Discovery, Error> {
        if let Some((d, at)) = &self.cache.lock().await.discovery
            && at.elapsed() < METADATA_LIFETIME
        {
            return Ok(d.clone());
        }
        let issuer = self.config.issuer.as_str().trim_end_matches('/');
        let url = format!("{issuer}/.well-known/openid-configuration");
        let found: Discovery = self.get_json(&url).await?;
        if found.issuer.trim_end_matches('/') != issuer {
            return Err(unavailable(format!(
                "the discovery document names issuer {}, not {issuer}",
                found.issuer
            )));
        }
        self.cache.lock().await.discovery = Some((found.clone(), Instant::now()));
        Ok(found)
    }

    async fn keys(&self, discovery: &Discovery, refresh: bool) -> Result<Vec<Jwk>, Error> {
        {
            let cache = self.cache.lock().await;
            if let Some((keys, at)) = &cache.keys {
                let fresh = at.elapsed() < METADATA_LIFETIME;
                if (fresh && !refresh) || at.elapsed() < JWKS_REFRESH_GAP {
                    return Ok(keys.clone());
                }
            }
        }
        let set: JwkSet = self.get_json(&discovery.jwks_uri).await?;
        self.cache.lock().await.keys = Some((set.keys.clone(), Instant::now()));
        Ok(set.keys)
    }

    /// Starts a sign-in that returns to `redirect_uri`, then to `next`.
    /// With `reauth`, the provider is asked to check the user again even
    /// if they are signed in there, as admin actions need a recent sign-in.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unavailable`] when the provider cannot be reached or
    /// too many sign-ins are in flight.
    pub async fn start(
        &self,
        redirect_uri: &str,
        next: Option<&str>,
        reauth: bool,
    ) -> Result<SsoStart, Error> {
        let discovery = self.discovery().await?;
        let state = random()?;
        let nonce = random()?;
        let verifier = random()?;
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        {
            let mut pending = self.pending.lock().await;
            pending.retain(|_, p| p.started.elapsed() < PENDING_LIFETIME);
            if pending.len() >= MAX_PENDING {
                return Err(Error::new(
                    ErrorKind::Unavailable,
                    "too many sign-ins in progress; try again in a few minutes",
                ));
            }
            let _ = pending.insert(
                state.clone(),
                Pending {
                    nonce: nonce.clone(),
                    verifier,
                    next: safe_next(next),
                    reauth,
                    started: Instant::now(),
                },
            );
        }
        let mut url = Url::parse(&discovery.authorization_endpoint)
            .map_err(|e| unavailable(format!("bad authorization endpoint: {e}")))?;
        let _ = url
            .query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &self.config.client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", "openid email profile")
            .append_pair("state", &state)
            .append_pair("nonce", &nonce)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256");
        if reauth {
            let _ = url
                .query_pairs_mut()
                .append_pair("prompt", "login")
                .append_pair("max_age", "0");
        }
        Ok(SsoStart {
            url: url.into(),
            state,
        })
    }

    /// Finishes a sign-in: `state` and `code` from the callback, and the
    /// state cookie the browser sent back.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for an unknown, expired or forged
    /// sign-in or token; [`ErrorKind::Unavailable`] when the provider
    /// cannot be reached.
    pub async fn finish(
        &self,
        redirect_uri: &str,
        state: &str,
        code: &str,
        cookie_state: Option<&str>,
    ) -> Result<SsoFinish, Error> {
        let same_browser =
            cookie_state.is_some_and(|c| bool::from(c.as_bytes().ct_eq(state.as_bytes())));
        if !same_browser {
            return Err(failed(
                "this sign-in was started in another browser or has expired; start again",
            ));
        }
        let pending = self
            .pending
            .lock()
            .await
            .remove(state)
            .filter(|p| p.started.elapsed() < PENDING_LIFETIME)
            .ok_or_else(|| failed("this sign-in has expired or was already used; start again"))?;
        let discovery = self.discovery().await?;
        let mut form = vec![
            ("grant_type".to_owned(), "authorization_code".to_owned()),
            ("code".to_owned(), code.to_owned()),
            ("redirect_uri".to_owned(), redirect_uri.to_owned()),
            ("code_verifier".to_owned(), pending.verifier.clone()),
        ];
        // client_secret_basic: both parts form-encoded (RFC 6749, 2.3.1).
        let basic = self.config.client_secret.as_ref().map(|secret| {
            let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect();
            (enc(&self.config.client_id), enc(secret))
        });
        if basic.is_none() {
            form.push(("client_id".to_owned(), self.config.client_id.clone()));
        }
        let (status, body) = self
            .http
            .post_form(&discovery.token_endpoint, form, basic)
            .await
            .map_err(unavailable)?;
        let tokens: Tokens = serde_json::from_slice(&body)
            .map_err(|_| unavailable(format!("unreadable token response (HTTP {status})")))?;
        if let Some(error) = tokens.error {
            let detail = tokens.error_description.unwrap_or_default();
            return Err(failed(format!(
                "the provider refused the sign-in: {error} {detail}"
            )));
        }
        let id_token = tokens
            .id_token
            .ok_or_else(|| failed("the provider sent no ID token"))?;
        let identity = self
            .verify(&discovery, &id_token, &pending, Timestamp::now())
            .await?;
        Ok(SsoFinish {
            identity,
            next: pending.next,
        })
    }

    async fn verify(
        &self,
        discovery: &Discovery,
        token: &str,
        pending: &Pending,
        now: Timestamp,
    ) -> Result<SsoIdentity, Error> {
        let parts: Vec<&str> = token.split('.').collect();
        let [header_b64, claims_b64, signature_b64] = parts.as_slice() else {
            return Err(failed("the ID token is malformed"));
        };
        let decode = |part: &str| {
            URL_SAFE_NO_PAD
                .decode(part.trim_end_matches('='))
                .map_err(|_| failed("the ID token is malformed"))
        };
        let header: Header = serde_json::from_slice(&decode(header_b64)?)
            .map_err(|_| failed("the ID token header is malformed"))?;
        let signature = decode(signature_b64)?;
        let signed = format!("{header_b64}.{claims_b64}");
        let mut keys = self.keys(discovery, false).await?;
        if find_key(&keys, &header.alg, header.kid.as_deref()).is_none() {
            // The provider may have rotated its keys.
            keys = self.keys(discovery, true).await?;
        }
        let key = find_key(&keys, &header.alg, header.kid.as_deref())
            .ok_or_else(|| failed("the ID token is signed with an unknown key"))?;
        check_signature(key, &header.alg, signed.as_bytes(), &signature)?;
        let claims: Claims = serde_json::from_slice(&decode(claims_b64)?)
            .map_err(|_| failed("the ID token claims are malformed"))?;
        claims.check(
            &discovery.issuer,
            &self.config.client_id,
            &pending.nonce,
            now,
        )?;
        if pending.reauth {
            claims.check_recent(now)?;
        }
        Ok(SsoIdentity {
            issuer: claims.iss,
            subject: claims.sub,
            email: claims.email.map(|e| e.trim().to_lowercase()),
            email_verified: claims.email_verified.is_some_and(|v| v.is_true()),
            name: claims
                .name
                .or(claims.preferred_username)
                .map(|n| n.trim().to_owned())
                .filter(|n| !n.is_empty()),
        })
    }
}

/// The token endpoint's answer.
#[derive(Deserialize)]
struct Tokens {
    id_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// A JWT header.
#[derive(Deserialize)]
struct Header {
    alg: String,
    #[serde(default)]
    kid: Option<String>,
}

/// `email_verified` is a boolean, but some providers send a string.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Flag {
    Bool(bool),
    Text(String),
}

impl Flag {
    fn is_true(&self) -> bool {
        match self {
            Self::Bool(b) => *b,
            Self::Text(t) => t.eq_ignore_ascii_case("true"),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    aud: Audience,
    #[serde(default)]
    azp: Option<String>,
    exp: i64,
    #[serde(default)]
    iat: Option<i64>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    auth_time: Option<i64>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_verified: Option<Flag>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    preferred_username: Option<String>,
}

impl Claims {
    /// A re-authentication must have checked the user just now, not
    /// reused a sign-in the provider remembered.
    fn check_recent(&self, now: Timestamp) -> Result<(), Error> {
        let now_secs = now.unix_ms() / 1000;
        let recent = self
            .auth_time
            .is_some_and(|at| (now_secs - at).abs() <= 5 * 60 + CLOCK_SKEW_SECS);
        if recent {
            Ok(())
        } else {
            Err(failed(
                "the provider did not confirm a fresh sign-in; sign in again",
            ))
        }
    }

    fn check(
        &self,
        issuer: &str,
        client_id: &str,
        nonce: &str,
        now: Timestamp,
    ) -> Result<(), Error> {
        if self.iss.trim_end_matches('/') != issuer.trim_end_matches('/') {
            return Err(failed("the ID token is from another issuer"));
        }
        let audiences: Vec<&str> = match &self.aud {
            Audience::One(a) => vec![a.as_str()],
            Audience::Many(list) => list.iter().map(String::as_str).collect(),
        };
        if !audiences.contains(&client_id) {
            return Err(failed("the ID token is for another application"));
        }
        if (audiences.len() > 1 || self.azp.is_some()) && self.azp.as_deref() != Some(client_id) {
            return Err(failed("the ID token was issued to another application"));
        }
        let now_secs = now.unix_ms() / 1000;
        if self.exp + CLOCK_SKEW_SECS <= now_secs {
            return Err(failed("the ID token has expired"));
        }
        if self.iat.is_some_and(|iat| iat - CLOCK_SKEW_SECS > now_secs) {
            return Err(failed("the ID token was issued in the future"));
        }
        let nonce_ok = self
            .nonce
            .as_deref()
            .is_some_and(|n| bool::from(n.as_bytes().ct_eq(nonce.as_bytes())));
        if !nonce_ok {
            return Err(failed("the ID token does not belong to this sign-in"));
        }
        if self.sub.is_empty() {
            return Err(failed("the ID token names no user"));
        }
        Ok(())
    }
}

fn find_key<'a>(keys: &'a [Jwk], alg: &str, kid: Option<&str>) -> Option<&'a Jwk> {
    let kty = match alg {
        "RS256" => "RSA",
        "ES256" => "EC",
        _ => return None,
    };
    let usable = |k: &&Jwk| {
        k.kty == kty
            && k.usage.as_deref().is_none_or(|u| u == "sig")
            && k.alg.as_deref().is_none_or(|a| a == alg)
    };
    if let Some(kid) = kid {
        return keys
            .iter()
            .filter(usable)
            .find(|k| k.kid.as_deref() == Some(kid));
    }
    // Without a key ID, only an unambiguous key will do.
    let mut candidates = keys.iter().filter(usable);
    let first = candidates.next();
    if candidates.next().is_some() {
        None
    } else {
        first
    }
}

fn check_signature(key: &Jwk, alg: &str, message: &[u8], signature: &[u8]) -> Result<(), Error> {
    let bad_key = || failed("the provider's signing key is malformed");
    let part = |v: &Option<String>| {
        v.as_deref()
            .and_then(|s| URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok())
            .ok_or_else(bad_key)
    };
    let verified = match alg {
        "RS256" => {
            let (n, e) = (part(&key.n)?, part(&key.e)?);
            RsaPublicKeyComponents { n: &n, e: &e }
                .verify(&RSA_PKCS1_2048_8192_SHA256, message, signature)
                .is_ok()
        }
        "ES256" => {
            if key.crv.as_deref() != Some("P-256") {
                return Err(bad_key());
            }
            let (x, y) = (part(&key.x)?, part(&key.y)?);
            if x.len() != 32 || y.len() != 32 {
                return Err(bad_key());
            }
            let mut point = Vec::with_capacity(65);
            point.push(0x04);
            point.extend_from_slice(&x);
            point.extend_from_slice(&y);
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &point)
                .verify(message, signature)
                .is_ok()
        }
        _ => false,
    };
    if verified {
        Ok(())
    } else {
        Err(failed("the ID token signature is not valid"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_stays_on_this_site() {
        assert_eq!(safe_next(Some("/drive?x=1")), "/drive?x=1");
        assert_eq!(safe_next(Some("//evil.example")), "/");
        assert_eq!(safe_next(Some("/\\evil.example")), "/");
        assert_eq!(safe_next(Some("https://evil.example")), "/");
        assert_eq!(safe_next(Some("/a\nb")), "/");
        assert_eq!(safe_next(None), "/");
    }

    #[test]
    fn accounts_are_created_only_for_allowed_domains() {
        let mut config = SsoConfig {
            issuer: Url::parse("https://id.northstar.dev").unwrap(),
            client_id: "vellum".into(),
            client_secret: Some("s".into()),
            display_name: "Northstar".into(),
            allowed_domains: vec!["northstar.dev".into()],
            auto_create: true,
        };
        assert!(config.may_create("ana@northstar.dev"));
        assert!(config.may_create("ana@NORTHSTAR.dev"));
        assert!(!config.may_create("ana@evil.dev"));
        assert!(!config.may_create("ana@sub.northstar.dev"));
        config.allowed_domains.clear();
        assert!(config.may_create("ana@evil.dev"));
        config.auto_create = false;
        assert!(!config.may_create("ana@northstar.dev"));
        assert!(!format!("{config:?}").contains("\"s\""));
    }

    #[test]
    fn only_rs256_and_es256_keys_are_used() {
        let rsa = Jwk {
            kty: "RSA".into(),
            kid: Some("a".into()),
            usage: Some("sig".into()),
            alg: None,
            n: None,
            e: None,
            crv: None,
            x: None,
            y: None,
        };
        let enc = Jwk {
            kid: Some("b".into()),
            usage: Some("enc".into()),
            ..rsa.clone()
        };
        let keys = vec![rsa, enc];
        assert!(find_key(&keys, "RS256", Some("a")).is_some());
        assert!(find_key(&keys, "RS256", Some("b")).is_none());
        assert!(find_key(&keys, "HS256", Some("a")).is_none());
        assert!(find_key(&keys, "none", None).is_none());
        assert!(find_key(&keys, "RS256", None).is_some());
    }
}
