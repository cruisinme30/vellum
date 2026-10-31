// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Signing in through a SAML provider set in Admin → Sign-in.
//!
//! ADR 0069, D293. The protocol checks are in [`crate::saml`]; this keeps
//! sign-ins in progress and used assertions in the replicated store, so any
//! replica can finish a sign-in and none accepts an assertion twice.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use vellum_kernel::{Error, ErrorKind, Timestamp};

use crate::identity::{Identity, NewSession, after};
use crate::providers::{
    MetadataUrl, SamlMetadataView, check_issuer, may_create, may_link,
};
use crate::saml::{self, Expected, IdentityProvider, ServiceProvider};
use crate::sso::{SsoIdentity, safe_next};
use crate::store::{Protocol, Provider, SamlRequest};
use crate::xmldsig::Certificate;

/// Where providers post responses, under the `/saml` mount.
pub const SAML_ACS_PATH: &str = "/saml/acs";
/// Vellum's SAML metadata, which is also its entity ID.
pub const SAML_METADATA_PATH: &str = "/saml/metadata";
/// How long a sign-in may take at the provider.
pub const SAML_PENDING: Duration = Duration::from_mins(10);
/// The most sign-ins in progress at once.
const MAX_PENDING: usize = 1000;

/// A started SAML sign-in: send the browser to `url` and keep
/// `relay_state` in a cookie.
#[derive(Debug, Clone)]
pub struct SamlStart {
    /// The provider's sign-in URL with the request.
    pub url: String,
    /// The relay state, also the cookie value that binds the browser.
    pub relay_state: String,
}

fn refused(message: &str) -> Error {
    Error::new(ErrorKind::Unauthenticated, message.to_owned())
}

fn hashed(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    URL_SAFE_NO_PAD.encode(hash.finalize())
}

fn identity_provider(record: &Provider) -> Result<IdentityProvider, Error> {
    let Protocol::Saml {
        entity_id,
        sso_url,
        certificates,
        ..
    } = &record.protocol
    else {
        return Err(Error::new(
            ErrorKind::NotFound,
            "this sign-in provider does not use SAML",
        ));
    };
    let certificates = certificates
        .iter()
        .map(|c| Certificate::parse_all(c))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    Ok(IdentityProvider {
        entity_id: entity_id.clone(),
        sso_url: sso_url.clone(),
        certificates,
    })
}

impl Identity {
    /// Vellum's side of SAML: its entity ID and response address.
    #[must_use]
    pub fn saml_service(&self) -> ServiceProvider {
        let base = self.config().base();
        ServiceProvider {
            entity_id: format!("{base}{SAML_METADATA_PATH}"),
            acs_url: format!("{base}{SAML_ACS_PATH}"),
        }
    }

    /// Whether `id` names an enabled SAML provider.
    #[must_use]
    pub fn is_saml_provider(&self, id: &str) -> bool {
        self.store().read(|s| {
            s.sign_in.providers.iter().any(|p| {
                p.id == id
                    && p.enabled
                    && matches!(p.protocol, Protocol::Saml { .. })
            })
        })
    }

    fn saml_record(&self, id: &str) -> Result<Provider, Error> {
        self.store()
            .read(|s| {
                s.sign_in
                    .providers
                    .iter()
                    .find(|p| p.id == id && p.enabled)
                    .cloned()
            })
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::NotFound,
                    "this sign-in provider is not set up",
                )
            })
    }

    /// Starts a sign-in at the SAML provider `id`, returning afterwards
    /// to `next`. `force` asks the provider to check the person again.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::NotFound`] for an unknown or disabled provider, and
    /// [`ErrorKind::Unavailable`] when too many sign-ins are in progress.
    pub fn saml_start(
        &self,
        id: &str,
        next: Option<&str>,
        force: bool,
    ) -> Result<SamlStart, Error> {
        let record = self.saml_record(id)?;
        let idp = identity_provider(&record)?;
        let request_id = saml::new_id()?;
        let relay_state = saml::new_id()?;
        let now = Timestamp::now();
        let request = SamlRequest {
            provider: record.id,
            request_id: request_id.clone(),
            next: safe_next(next),
            expires: after(now, SAML_PENDING),
            reauth: force,
        };
        self.store().write(|s| {
            if s.saml_requests.len() >= MAX_PENDING {
                s.saml_requests.retain(|_, r| r.expires > now);
            }
            if s.saml_requests.len() >= MAX_PENDING {
                return Err(Error::new(
                    ErrorKind::Unavailable,
                    "too many sign-ins in progress; try again in a few minutes",
                ));
            }
            let _ = s.saml_requests.insert(hashed(&[&relay_state]), request);
            Ok(())
        })??;
        let url = saml::authn_request_url(
            &self.saml_service(),
            &idp,
            &request_id,
            &relay_state,
            force,
            now,
        )?;
        Ok(SamlStart { url, relay_state })
    }

    /// Finishes a SAML sign-in: the posted `SAMLResponse` and
    /// `RelayState`, and the relay state cookie the browser sent. Returns
    /// the new session and where to go.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unauthenticated`] for a sign-in started elsewhere,
    /// expired or already finished, a response that fails any check, or a
    /// replayed assertion; otherwise as [`Identity::sso_sign_in`].
    pub fn saml_finish(
        &self,
        relay_state: &str,
        cookie: Option<&str>,
        response: &str,
        user_agent: Option<&str>,
    ) -> Result<(NewSession, String), Error> {
        let same_browser = cookie.is_some_and(|c| {
            bool::from(c.as_bytes().ct_eq(relay_state.as_bytes()))
        });
        if !same_browser {
            return Err(refused(
                "this sign-in was started in another browser or has expired; \
                    start again",
            ));
        }
        let now = Timestamp::now();
        let request = self
            .store()
            .write(|s| s.saml_requests.remove(&hashed(&[relay_state])))?
            .filter(|r| r.expires > now)
            .ok_or_else(|| {
                refused(
                    "this sign-in has expired or was already used; start again",
                )
            })?;
        let record = self.saml_record(&request.provider)?;
        let idp = identity_provider(&record)?;
        let sp = self.saml_service();
        let asserted = saml::read_response(
            response,
            &Expected {
                sp: &sp,
                idp: &idp,
                request_id: &request.request_id,
                now,
            },
        )
        .inspect_err(|e| {
            tracing::info!(
                event = "identity.saml_refused",
                provider = %record.id,
                error = %e,
                "a SAML response was refused"
            );
        })?;
        if request.reauth {
            saml::check_fresh(&asserted, now).inspect_err(|e| {
                tracing::info!(
                    event = "identity.saml_refused",
                    provider = %record.id,
                    error = %e,
                    "a SAML re-authentication was not fresh"
                );
            })?;
        }
        let seen = hashed(&[&record.id, &asserted.id]);
        self.store().write(|s| {
            if s.saml_seen.get(&seen).is_some_and(|until| *until > now) {
                return Err(refused("this sign-in response was already used"));
            }
            let _ = s.saml_seen.insert(seen.clone(), asserted.expires);
            Ok(())
        })??;
        let found = SsoIdentity {
            issuer: asserted.issuer.clone(),
            subject: asserted.name_id.clone(),
            email_verified: asserted.email.is_some(),
            email: asserted.email.clone(),
            name: asserted.name,
        };
        let session = self.federated_sign_in(
            &found,
            &|email| may_create(&record, email),
            &|email| may_link(&record, email),
            user_agent,
        )?;
        Ok((session, request.next))
    }

    /// Reads a provider's SAML metadata from `url`, through the SSRF
    /// guard, to fill in Admin → Sign-in.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] for a bad URL or document, and
    /// [`ErrorKind::Unavailable`] when it cannot be fetched.
    pub async fn read_saml_metadata(
        &self,
        input: &MetadataUrl,
    ) -> Result<SamlMetadataView, Error> {
        let url = check_issuer(&input.url)?;
        let factory = self.sso_http_factory().ok_or_else(|| {
            Error::new(
                ErrorKind::Unavailable,
                "this server cannot call out to read metadata",
            )
        })?;
        let http = factory(&url)?;
        let bytes = http.get(url.as_str()).await?;
        let metadata = saml::parse_metadata(&bytes)?;
        Ok(SamlMetadataView {
            entity_id: metadata.entity_id,
            sso_url: metadata.sso_url,
            certificate: metadata
                .certificates
                .iter()
                .map(Certificate::to_pem)
                .collect(),
            certificate_fingerprints: metadata
                .certificates
                .iter()
                .map(Certificate::fingerprint)
                .collect(),
        })
    }
}
