// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Identity providers set in Admin → Sign-in (ADR 0069, D292).
//!
//! Single sign-on used to be one `OpenID Connect` provider read from
//! `VELLUM_OIDC_*` at start-up. Admins now add any number of providers;
//! each is a record in the replicated sign-in settings, with its client
//! secret sealed under the identity key and never returned. The provider
//! from the environment still works, with the ID [`ENV_PROVIDER`], and
//! shows as read-only.
//!
//! A provider's relying party ([`Sso`]) is built on first use and kept
//! until its settings change, so in-flight sign-ins and the discovery and
//! key caches survive unrelated edits.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use url::Url;
use vellum_contracts::{HttpMethod, Operation, RequestContext, Scope};
use vellum_kernel::{Error, ErrorKind, Timestamp};

use crate::identity::Identity;
use crate::sso::{Sso, SsoConfig, SsoHttp};
use crate::store::{Protocol, Provider, SecondFactorPolicy};

/// The ID of the provider configured by `VELLUM_OIDC_*`.
pub const ENV_PROVIDER: &str = "env";
/// The most providers a workspace keeps.
const MAX_PROVIDERS: usize = 20;
/// How recent a sign-in provider changes need.
const FRESH_AUTH: Duration = crate::service::FRESH_AUTH;

/// Builds the HTTP client a provider's sign-ins call out through (the
/// SSRF guard, trusting the issuer's host).
pub type SsoHttpFactory = Arc<dyn Fn(&Url) -> Result<Arc<dyn SsoHttp>, Error> + Send + Sync>;

/// Relying parties built from provider records, by ID, with the settings
/// they were built from.
#[derive(Default)]
pub(crate) struct SsoCache(std::sync::Mutex<HashMap<String, (Provider, Arc<Sso>)>>);

impl std::fmt::Debug for SsoCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SsoCache(..)")
    }
}

/// A provider as Admin → Sign-in shows it. Secrets are never returned.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent flags an admin reads"
)]
pub struct ProviderView {
    /// The provider's ID.
    pub id: String,
    /// The button label.
    pub name: String,
    /// `oidc` or `saml`.
    pub kind: String,
    /// Whether the sign-in page offers it.
    pub enabled: bool,
    /// For OIDC, the issuer URL.
    pub issuer: Option<String>,
    /// For OIDC, the client ID.
    pub client_id: Option<String>,
    /// For OIDC, whether a client secret is stored.
    pub has_client_secret: bool,
    /// Email domains whose people get an account on first sign-in.
    pub allowed_domains: Vec<String>,
    /// Whether a verified address with no account gets one.
    pub auto_create: bool,
    /// Set by environment variables: shown, not changed here.
    pub read_only: bool,
    /// Where its sign-in button goes.
    pub start_url: String,
}

/// A provider an admin adds or changes.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ProviderInput {
    /// The button label, 1 to 60 characters.
    pub name: String,
    /// Whether the sign-in page offers it.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The issuer URL (`https`, or `http` on this machine for testing).
    pub issuer: String,
    /// The client ID registered with the provider.
    pub client_id: String,
    /// A new client secret; absent keeps the stored one, empty removes it.
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Email domains whose people get an account on first sign-in.
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// Whether a verified address with no account gets one.
    #[serde(default)]
    pub auto_create: bool,
}

const fn yes() -> bool {
    true
}

/// A changed provider, with its ID in the path.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateProvider {
    /// The provider's ID.
    pub id: String,
    /// Its new settings.
    pub provider: ProviderInput,
}

/// A provider, by ID in the path.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProviderId {
    /// The provider's ID.
    pub id: String,
}

/// The workspace's sign-in rules, as Admin → Sign-in shows them.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SignInView {
    /// Who must use a second factor with a password.
    pub second_factor: SecondFactorPolicy,
    /// The longest expiry of new API tokens, in days, if capped.
    pub max_token_days: Option<u32>,
    /// Whether passwords are refused, except for break-glass admins.
    pub require_sso: bool,
    /// The identity providers, the environment's first.
    pub providers: Vec<ProviderView>,
    /// The redirect URI to register with OIDC providers.
    pub oidc_callback_url: String,
    /// How many enabled admins may still use a password (break-glass).
    pub break_glass_admins: usize,
}

/// Changes to the sign-in rules; absent fields stay as they are.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct UpdateSignIn {
    /// Who must use a second factor with a password.
    pub second_factor: Option<SecondFactorPolicy>,
    /// Refuse passwords, except for break-glass admins.
    pub require_sso: Option<bool>,
}

/// A sign-in button.
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SsoButton {
    /// The provider's ID.
    pub id: String,
    /// The provider's name, for "Sign in with {name}".
    pub name: String,
    /// Where the button goes; append `&next=<path>` to return there.
    pub start_url: String,
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message.into())
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// Checks an issuer URL: `https`, or `http` on this machine.
///
/// # Errors
///
/// [`ErrorKind::InvalidInput`] for anything else.
pub fn check_issuer(text: &str) -> Result<Url, Error> {
    let url =
        Url::parse(text.trim()).map_err(|e| invalid(format!("the issuer is not a URL: {e}")))?;
    let local_http = url.scheme() == "http" && is_loopback(&url);
    if url.scheme() != "https" && !local_http {
        return Err(invalid(
            "the issuer must use https (http only on this machine)",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(invalid("the issuer cannot hold credentials or a fragment"));
    }
    Ok(url)
}

/// Cleans a domain list: lower-case, without `@`, no empties or repeats.
fn domains(list: &[String]) -> Result<Vec<String>, Error> {
    let mut out: Vec<String> = Vec::new();
    for d in list {
        let d = d.trim().trim_start_matches('@').to_lowercase();
        if d.is_empty() {
            continue;
        }
        if !d.contains('.')
            || d.len() > 253
            || !d
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        {
            return Err(invalid(format!("{d} is not a domain")));
        }
        if !out.contains(&d) {
            out.push(d);
        }
    }
    Ok(out)
}

fn check_provider_name(name: &str) -> Result<String, Error> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err(invalid("name the provider in 1 to 60 characters"));
    }
    Ok(name.to_owned())
}

/// A URL-safe ID from a name, unique among `taken`.
fn slug(name: &str, taken: &[&str]) -> String {
    let mut base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    base.truncate(30);
    if base.is_empty() || base == ENV_PROVIDER {
        "provider".clone_into(&mut base);
    }
    if !taken.contains(&base.as_str()) {
        return base;
    }
    (2..=MAX_PROVIDERS + 1)
        .map(|n| format!("{base}-{n}"))
        .find(|c| !taken.contains(&c.as_str()))
        .unwrap_or(base)
}

/// The associated data a provider's client secret is sealed with.
fn secret_purpose(id: &str) -> String {
    format!("oidc-client-secret:{id}")
}

fn start_url(id: &str) -> String {
    format!(
        "/auth/sso/start?{}",
        serde_urlencoded::to_string([("provider", id)]).unwrap_or_default()
    )
}

fn env_view(sso: &Sso) -> ProviderView {
    let config = sso.config();
    ProviderView {
        id: ENV_PROVIDER.to_owned(),
        name: config.display_name.clone(),
        kind: "oidc".to_owned(),
        enabled: true,
        issuer: Some(config.issuer.to_string()),
        client_id: Some(config.client_id.clone()),
        has_client_secret: config.client_secret.is_some(),
        allowed_domains: config.allowed_domains.clone(),
        auto_create: config.auto_create,
        read_only: true,
        start_url: start_url(ENV_PROVIDER),
    }
}

fn record_view(p: &Provider) -> ProviderView {
    let Protocol::Oidc {
        issuer,
        client_id,
        client_secret,
    } = &p.protocol;
    ProviderView {
        id: p.id.clone(),
        name: p.name.clone(),
        kind: "oidc".to_owned(),
        enabled: p.enabled,
        issuer: Some(issuer.clone()),
        client_id: Some(client_id.clone()),
        has_client_secret: client_secret.is_some(),
        allowed_domains: p.allowed_domains.clone(),
        auto_create: p.auto_create,
        read_only: false,
        start_url: start_url(&p.id),
    }
}

impl Identity {
    /// The sign-in buttons: the environment's provider, then every enabled
    /// one from Admin → Sign-in.
    #[must_use]
    pub fn sign_in_buttons(&self) -> Vec<SsoButton> {
        let mut buttons: Vec<SsoButton> = self
            .sso()
            .map(|sso| SsoButton {
                id: ENV_PROVIDER.to_owned(),
                name: sso.config().display_name.clone(),
                start_url: start_url(ENV_PROVIDER),
            })
            .into_iter()
            .collect();
        buttons.extend(self.store().read(|s| {
            s.sign_in
                .providers
                .iter()
                .filter(|p| p.enabled)
                .map(|p| SsoButton {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    start_url: start_url(&p.id),
                })
                .collect::<Vec<_>>()
        }));
        buttons
    }

    /// The relying party for a provider: `id`, or with no ID the only
    /// one (the environment's first).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown or disabled provider, and
    /// [`ErrorKind::Unavailable`] when no HTTP client is set up or the
    /// client secret cannot be opened.
    pub fn sso_provider(&self, id: Option<&str>) -> Result<(String, Arc<Sso>), Error> {
        let missing = || Error::new(ErrorKind::NotFound, "this sign-in provider is not set up");
        let id = match id.filter(|i| !i.is_empty()) {
            Some(id) => id.to_owned(),
            None => self
                .sign_in_buttons()
                .into_iter()
                .next()
                .map(|b| b.id)
                .ok_or_else(missing)?,
        };
        if id == ENV_PROVIDER {
            return self.sso().cloned().map(|sso| (id, sso)).ok_or_else(missing);
        }
        let record = self
            .store()
            .read(|s| {
                s.sign_in
                    .providers
                    .iter()
                    .find(|p| p.id == id && p.enabled)
                    .cloned()
            })
            .ok_or_else(missing)?;
        let mut built = self
            .sso_cache()
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some((seen, sso)) = built.get(&id)
            && seen == &record
        {
            let sso = Arc::clone(sso);
            drop(built);
            return Ok((id, sso));
        }
        let sso = Arc::new(self.build_sso(&record)?);
        let _ = built.insert(id.clone(), (record, Arc::clone(&sso)));
        drop(built);
        Ok((id, sso))
    }

    fn build_sso(&self, record: &Provider) -> Result<Sso, Error> {
        let Protocol::Oidc {
            issuer,
            client_id,
            client_secret,
        } = &record.protocol;
        let issuer = check_issuer(issuer)?;
        let client_secret = client_secret
            .as_deref()
            .map(|sealed| {
                let bytes = self.sealer()?.open(&secret_purpose(&record.id), sealed)?;
                String::from_utf8(bytes)
                    .map_err(|_| Error::new(ErrorKind::Internal, "a client secret is not text"))
            })
            .transpose()?;
        let factory = self.sso_http_factory().ok_or_else(|| {
            Error::new(
                ErrorKind::Unavailable,
                "single sign-on cannot call out from this server",
            )
        })?;
        let http = factory(&issuer)?;
        Ok(Sso::new(
            SsoConfig {
                issuer,
                client_id: client_id.clone(),
                client_secret,
                display_name: record.name.clone(),
                allowed_domains: record.allowed_domains.clone(),
                auto_create: record.auto_create,
            },
            http,
        ))
    }

    /// The sign-in rules and providers, for Admin → Sign-in.
    #[must_use]
    pub fn sign_in_settings(&self) -> SignInView {
        let mut providers: Vec<ProviderView> =
            self.sso().map(|s| env_view(s)).into_iter().collect();
        let (settings, break_glass_admins) = self.store().read(|s| {
            (
                s.sign_in.clone(),
                s.users
                    .values()
                    .filter(|u| u.admin && u.break_glass && !u.disabled)
                    .count(),
            )
        });
        providers.extend(settings.providers.iter().map(record_view));
        SignInView {
            second_factor: settings.second_factor,
            max_token_days: settings.max_token_days,
            require_sso: settings.require_sso,
            providers,
            oidc_callback_url: format!(
                "{}{}",
                self.config().base(),
                crate::http::SSO_CALLBACK_PATH
            ),
            break_glass_admins,
        }
    }

    /// Changes the sign-in rules.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Conflict`] when single sign-on would be required with
    /// no provider to sign in with.
    pub fn update_sign_in(&self, change: &UpdateSignIn) -> Result<SignInView, Error> {
        let has_env = self.sso().is_some();
        self.store().write(|s| {
            if change.require_sso == Some(true)
                && !has_env
                && !s.sign_in.providers.iter().any(|p| p.enabled)
            {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "add and turn on a sign-in provider before requiring single sign-on",
                ));
            }
            if let Some(policy) = change.second_factor {
                s.sign_in.second_factor = policy;
            }
            if let Some(require) = change.require_sso {
                s.sign_in.require_sso = require;
            }
            Ok(())
        })??;
        tracing::info!(
            event = "identity.sign_in_changed",
            "sign-in settings changed"
        );
        Ok(self.sign_in_settings())
    }

    fn checked_protocol(
        &self,
        id: &str,
        input: &ProviderInput,
        old_secret: Option<String>,
    ) -> Result<Protocol, Error> {
        let issuer = check_issuer(&input.issuer)?;
        let client_id = input.client_id.trim();
        if client_id.is_empty() || client_id.len() > 255 {
            return Err(invalid("enter the client ID from the provider"));
        }
        let client_secret = match input.client_secret.as_deref() {
            None => old_secret,
            Some("") => None,
            Some(secret) => {
                if secret.len() > 1024 {
                    return Err(invalid("the client secret is too long"));
                }
                Some(
                    self.sealer()?
                        .seal(&secret_purpose(id), secret.as_bytes())?,
                )
            }
        };
        Ok(Protocol::Oidc {
            issuer: issuer.to_string(),
            client_id: client_id.to_owned(),
            client_secret,
        })
    }

    /// Adds an OIDC provider.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] for bad settings, and
    /// [`ErrorKind::Conflict`] at the provider limit.
    pub fn add_provider(&self, input: &ProviderInput) -> Result<ProviderView, Error> {
        let name = check_provider_name(&input.name)?;
        let allowed_domains = domains(&input.allowed_domains)?;
        let id = self.store().read(|s| {
            let taken: Vec<&str> = s.sign_in.providers.iter().map(|p| p.id.as_str()).collect();
            slug(&name, &taken)
        });
        let protocol = self.checked_protocol(&id, input, None)?;
        let record = Provider {
            id: id.clone(),
            name,
            enabled: input.enabled,
            allowed_domains,
            auto_create: input.auto_create,
            created_at: Timestamp::now(),
            protocol,
        };
        let stored = record.clone();
        self.store().write(|s| {
            if s.sign_in.providers.len() >= MAX_PROVIDERS {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    format!("a workspace keeps at most {MAX_PROVIDERS} providers"),
                ));
            }
            if s.sign_in.providers.iter().any(|p| p.id == stored.id) {
                return Err(Error::new(ErrorKind::Conflict, "try again"));
            }
            s.sign_in.providers.push(stored);
            Ok(())
        })??;
        tracing::info!(event = "identity.provider_added", provider = %id, "sign-in provider added");
        Ok(record_view(&record))
    }

    /// Changes a provider. An absent client secret keeps the stored one.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown provider (the environment's
    /// cannot be changed here), and [`ErrorKind::InvalidInput`] for bad
    /// settings.
    pub fn update_provider(&self, id: &str, input: &ProviderInput) -> Result<ProviderView, Error> {
        let name = check_provider_name(&input.name)?;
        let allowed_domains = domains(&input.allowed_domains)?;
        let old = self
            .store()
            .read(|s| s.sign_in.providers.iter().find(|p| p.id == id).cloned())
            .ok_or_else(|| Error::new(ErrorKind::NotFound, "provider not found"))?;
        let Protocol::Oidc { client_secret, .. } = old.protocol;
        let protocol = self.checked_protocol(id, input, client_secret)?;
        let updated = self.store().write(|s| {
            let p = s.sign_in.providers.iter_mut().find(|p| p.id == id)?;
            p.name = name;
            p.enabled = input.enabled;
            p.allowed_domains = allowed_domains;
            p.auto_create = input.auto_create;
            p.protocol = protocol;
            Some(p.clone())
        })?;
        let updated =
            updated.ok_or_else(|| Error::new(ErrorKind::NotFound, "provider not found"))?;
        tracing::info!(event = "identity.provider_changed", provider = %id, "sign-in provider changed");
        Ok(record_view(&updated))
    }

    /// Removes a provider. People linked to it keep their accounts and
    /// sessions, and can still sign in another way.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown provider, and
    /// [`ErrorKind::Conflict`] when single sign-on is required and this is
    /// the last provider.
    pub fn remove_provider(&self, id: &str) -> Result<(), Error> {
        let has_env = self.sso().is_some();
        self.store().write(|s| {
            if !s.sign_in.providers.iter().any(|p| p.id == id) {
                return Err(Error::new(ErrorKind::NotFound, "provider not found"));
            }
            let others = has_env || s.sign_in.providers.iter().any(|p| p.id != id && p.enabled);
            if s.sign_in.require_sso && !others {
                return Err(Error::new(
                    ErrorKind::Conflict,
                    "single sign-on is required: turn that off or add another provider first",
                ));
            }
            s.sign_in.providers.retain(|p| p.id != id);
            Ok(())
        })??;
        let _ = self
            .sso_cache()
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id);
        tracing::info!(event = "identity.provider_removed", provider = %id, "sign-in provider removed");
        Ok(())
    }
}

/// The admin operations of Admin → Sign-in.
#[expect(
    clippy::literal_string_with_formatting_args,
    reason = "`{id}` is a path template, not a format string"
)]
pub(crate) fn operations(identity: &Arc<Identity>) -> Vec<Operation> {
    let settings = {
        let id = Arc::clone(identity);
        move |_: RequestContext, _: crate::service::Empty| {
            let id = Arc::clone(&id);
            async move { Ok(id.sign_in_settings()) }
        }
    };
    let update = {
        let id = Arc::clone(identity);
        move |_: RequestContext, input: UpdateSignIn| {
            let id = Arc::clone(&id);
            async move { id.update_sign_in(&input) }
        }
    };
    let add = {
        let id = Arc::clone(identity);
        move |_: RequestContext, input: ProviderInput| {
            let id = Arc::clone(&id);
            async move { id.add_provider(&input) }
        }
    };
    let change = {
        let id = Arc::clone(identity);
        move |_: RequestContext, input: UpdateProvider| {
            let id = Arc::clone(&id);
            async move { id.update_provider(&input.id, &input.provider) }
        }
    };
    let remove = {
        let id = Arc::clone(identity);
        move |_: RequestContext, input: ProviderId| {
            let id = Arc::clone(&id);
            async move {
                id.remove_provider(&input.id)?;
                Ok(id.sign_in_settings())
            }
        }
    };
    vec![
        Operation::query(
            "identity.admin_sign_in",
            "/v1/admin/sign-in",
            Scope::Admin,
            settings,
        )
        .fresh_auth(FRESH_AUTH)
        .summary("The workspace's sign-in rules and providers"),
        Operation::mutation(
            "identity.admin_update_sign_in",
            HttpMethod::Patch,
            "/v1/admin/sign-in",
            Scope::Admin,
            update,
        )
        .fresh_auth(FRESH_AUTH)
        .summary("Change the workspace's sign-in rules"),
        Operation::mutation(
            "identity.admin_add_provider",
            HttpMethod::Post,
            "/v1/admin/sign-in/providers",
            Scope::Admin,
            add,
        )
        .fresh_auth(FRESH_AUTH)
        .summary("Add an OpenID Connect sign-in provider"),
        Operation::mutation(
            "identity.admin_update_provider",
            HttpMethod::Put,
            "/v1/admin/sign-in/providers/{id}",
            Scope::Admin,
            change,
        )
        .fresh_auth(FRESH_AUTH)
        .summary("Change a sign-in provider"),
        Operation::mutation(
            "identity.admin_remove_provider",
            HttpMethod::Delete,
            "/v1/admin/sign-in/providers/{id}",
            Scope::Admin,
            remove,
        )
        .fresh_auth(FRESH_AUTH)
        .summary("Remove a sign-in provider"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_slugs_and_unique() {
        assert_eq!(slug("Okta (Northstar)", &[]), "okta-northstar");
        assert_eq!(slug("Okta", &["okta"]), "okta-2");
        assert_eq!(slug("Okta", &["okta", "okta-2"]), "okta-3");
        assert_eq!(slug("!!!", &[]), "provider");
        assert_eq!(slug("env", &[]), "provider");
    }

    #[test]
    fn issuers_and_domains_are_checked() {
        assert!(check_issuer("https://id.northstar.dev").is_ok());
        assert!(check_issuer("http://localhost:8080/realms/x").is_ok());
        assert!(check_issuer("http://id.northstar.dev").is_err());
        assert!(check_issuer("https://user:pw@id.northstar.dev").is_err());
        assert!(check_issuer("javascript:alert(1)").is_err());
        assert_eq!(
            domains(&["@Northstar.dev".into(), " ".into(), "northstar.dev".into()]).unwrap(),
            ["northstar.dev"]
        );
        assert!(domains(&["not a domain".into()]).is_err());
    }
}
