// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Vellum as a SAML 2.0 service provider (ADR 0069, D293).
//!
//! Sign-in starts at Vellum (the browser goes to the provider with a
//! deflated `AuthnRequest`), and the provider posts a `Response` back to
//! the assertion consumer service. A response is accepted only when:
//!
//! - it parses without a DTD (no entity expansion) and every element ID
//!   is unique;
//! - it holds exactly one `Assertion`, signed by itself with one of the
//!   provider's pinned certificates ([`crate::xmldsig`]); encrypted
//!   assertions are refused;
//! - the assertion answers this sign-in (`InResponseTo`), names Vellum as
//!   audience and recipient, comes from the provider's entity ID, and is
//!   inside its validity window (two minutes of clock skew).
//!
//! Every claim is read from the verified assertion element only. The
//! caller keeps each assertion ID until it expires, so it works once.

use std::fmt::Write as _;
use std::io::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use roxmltree::{Document, Node, ParsingOptions};
use url::Url;
use vellum_kernel::{Error, ErrorKind, Timestamp};

use crate::xmldsig::{self, Certificate, children, only_child, text_of};

/// The SAML protocol namespace.
pub const PROTOCOL_NS: &str = "urn:oasis:names:tc:SAML:2.0:protocol";
/// The SAML assertion namespace.
pub const ASSERTION_NS: &str = "urn:oasis:names:tc:SAML:2.0:assertion";
/// The SAML metadata namespace.
pub const METADATA_NS: &str = "urn:oasis:names:tc:SAML:2.0:metadata";
const SUCCESS: &str = "urn:oasis:names:tc:SAML:2.0:status:Success";
const BEARER: &str = "urn:oasis:names:tc:SAML:2.0:cm:bearer";
const HTTP_REDIRECT: &str =
    "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect";
const HTTP_POST: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST";
const EMAIL_FORMAT: &str =
    "urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress";

/// Clock difference tolerated between Vellum and the provider.
const CLOCK_SKEW_MS: i64 = 120_000;
/// The largest response read, before base64 decoding.
const MAX_RESPONSE_BYTES: usize = 512 * 1024;
/// The most XML nodes in a response or metadata document.
const MAX_NODES: u32 = 20_000;
/// How long a verified assertion is remembered at most, against replay.
const MAX_REMEMBER_MS: i64 = 24 * 60 * 60 * 1000;

/// Attribute names that carry the email address.
const EMAIL_ATTRIBUTES: &[&str] = &[
    "email",
    "mail",
    "emailaddress",
    "http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress",
    "urn:oid:0.9.2342.19200300.100.1.3",
];
/// Attribute names that carry the display name.
const NAME_ATTRIBUTES: &[&str] = &[
    "name",
    "displayname",
    "http://schemas.microsoft.com/identity/claims/displayname",
    "urn:oid:2.16.840.1.113730.3.1.241",
    "urn:oid:2.5.4.3",
];

fn refused(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Unauthenticated, message.into())
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message.into())
}

/// Vellum's side: its entity ID and where responses are posted.
#[derive(Debug, Clone)]
pub struct ServiceProvider {
    /// The entity ID (Vellum's metadata URL).
    pub entity_id: String,
    /// The assertion consumer service URL.
    pub acs_url: String,
}

/// The provider's side, from its metadata or typed in.
#[derive(Debug, Clone)]
pub struct IdentityProvider {
    /// The provider's entity ID, as its assertions name their issuer.
    pub entity_id: String,
    /// Where `AuthnRequest`s go (HTTP-Redirect binding).
    pub sso_url: String,
    /// The certificates its assertions may be signed with.
    pub certificates: Vec<Certificate>,
}

/// What a verified assertion says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asserted {
    /// The assertion's ID, remembered against replay.
    pub id: String,
    /// The provider's entity ID.
    pub issuer: String,
    /// The provider's name for the person.
    pub name_id: String,
    /// The email address, from an attribute or an email `NameID`.
    pub email: Option<String>,
    /// The display name, if shared.
    pub name: Option<String>,
    /// Until when the assertion could be replayed.
    pub expires: Timestamp,
    /// When the provider last checked the person (`AuthnInstant`).
    pub authn_instant: Option<Timestamp>,
}

/// How recent `AuthnInstant` must be for a re-authentication, as for
/// OIDC's `auth_time`.
const FRESH_MS: i64 = 5 * 60 * 1000;

/// Checks that a re-authentication's assertion says the provider checked
/// the person within the last five minutes (RT-046). Providers may ignore
/// `ForceAuthn` and answer from a remembered sign-in.
///
/// # Errors
///
/// [`ErrorKind::Unauthenticated`] without a recent `AuthnInstant`.
pub fn check_fresh(asserted: &Asserted, now: Timestamp) -> Result<(), Error> {
    let now = now.unix_ms();
    let fresh = asserted.authn_instant.is_some_and(|at| {
        let at = at.unix_ms();
        at <= now + CLOCK_SKEW_MS && now - at <= FRESH_MS + CLOCK_SKEW_MS
    });
    if fresh {
        Ok(())
    } else {
        Err(refused(
            "the provider did not confirm a fresh sign-in; sign in again",
        ))
    }
}

/// What a sign-in expects of the response.
#[derive(Debug, Clone, Copy)]
pub struct Expected<'a> {
    /// Vellum's side.
    pub sp: &'a ServiceProvider,
    /// The provider it went to.
    pub idp: &'a IdentityProvider,
    /// The `AuthnRequest` ID it answers.
    pub request_id: &'a str,
    /// Now.
    pub now: Timestamp,
}

/// The provider's settings as its metadata states them.
#[derive(Debug, Clone)]
pub struct Metadata {
    /// The entity ID.
    pub entity_id: String,
    /// The HTTP-Redirect sign-in URL.
    pub sso_url: String,
    /// The signing certificates.
    pub certificates: Vec<Certificate>,
}

/// A fresh request or relay ID: `_` and 40 hex characters.
///
/// # Errors
///
/// [`ErrorKind::Internal`] if the random source fails.
pub fn new_id() -> Result<String, Error> {
    let mut bytes = [0_u8; 20];
    getrandom::fill(&mut bytes).map_err(|e| {
        Error::new(ErrorKind::Internal, format!("random source failed: {e}"))
    })?;
    let mut id = String::from("_");
    for b in bytes {
        let _ = write!(id, "{b:02x}");
    }
    Ok(id)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The URL that sends the browser to the provider with an
/// `AuthnRequest` (HTTP-Redirect binding). `force` asks the provider to
/// check the person again, for re-authentication.
///
/// # Errors
///
/// [`ErrorKind::InvalidInput`] for a bad sign-in URL.
pub fn authn_request_url(
    sp: &ServiceProvider,
    idp: &IdentityProvider,
    request_id: &str,
    relay_state: &str,
    force: bool,
    now: Timestamp,
) -> Result<String, Error> {
    let force = if force { " ForceAuthn=\"true\"" } else { "" };
    let xml = format!(
        "<samlp:AuthnRequest xmlns:samlp=\"{PROTOCOL_NS}\" \
         xmlns:saml=\"{ASSERTION_NS}\" ID=\"{id}\" Version=\"2.0\" \
         IssueInstant=\"{now}\" Destination=\"{dest}\" \
         AssertionConsumerServiceURL=\"{acs}\" \
         ProtocolBinding=\"{HTTP_POST}\"{force}><saml:Issuer>{issuer}</saml:Iss\
         uer><samlp:NameIDPolicy AllowCreate=\"true\"/></samlp:AuthnRequest>",
        id = escape(request_id),
        dest = escape(&idp.sso_url),
        acs = escape(&sp.acs_url),
        issuer = escape(&sp.entity_id),
    );
    let mut deflate = DeflateEncoder::new(Vec::new(), Compression::default());
    deflate.write_all(xml.as_bytes()).map_err(|e| {
        Error::new(ErrorKind::Internal, format!("deflate failed: {e}"))
    })?;
    let deflated = deflate.finish().map_err(|e| {
        Error::new(ErrorKind::Internal, format!("deflate failed: {e}"))
    })?;
    let mut url = Url::parse(&idp.sso_url).map_err(|e| {
        invalid(format!("the provider's sign-in URL is not a URL: {e}"))
    })?;
    let _ = url
        .query_pairs_mut()
        .append_pair("SAMLRequest", &STANDARD.encode(deflated))
        .append_pair("RelayState", relay_state);
    Ok(url.into())
}

/// Vellum's service provider metadata, for the provider's admin.
#[must_use]
pub fn sp_metadata(sp: &ServiceProvider) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <md:EntityDescriptor xmlns:md=\"{METADATA_NS}\" \
         entityID=\"{entity}\"><md:SPSSODescriptor \
         AuthnRequestsSigned=\"false\" WantAssertionsSigned=\"true\" \
         protocolSupportEnumeration=\"{PROTOCOL_NS}\"><md:NameIDFormat>{EMAIL_F\
         ORMAT}</md:NameIDFormat><md:AssertionConsumerService \
         Binding=\"{HTTP_POST}\" Location=\"{acs}\" index=\"0\" \
         isDefault=\"true\"/></md:SPSSODescriptor></md:EntityDescriptor>\n",
        entity = escape(&sp.entity_id),
        acs = escape(&sp.acs_url),
    )
}

fn parse(text: &str) -> Result<Document<'_>, Error> {
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_NODES,
        ..ParsingOptions::default()
    };
    Document::parse_with_options(text, options).map_err(|e| {
        tracing::info!(event = "identity.saml_refused", error = %e, "unreadabl\
            e SAML XML");
        refused(format!("the XML cannot be read: {e}"))
    })
}

fn is(node: Node<'_, '_>, ns: &str, name: &str) -> bool {
    node.is_element()
        && node.tag_name().namespace() == Some(ns)
        && node.tag_name().name() == name
}

fn time(
    node: Node<'_, '_>,
    attribute: &str,
) -> Result<Option<Timestamp>, Error> {
    node.attribute(attribute)
        .map(|t| {
            t.parse::<Timestamp>()
                .map_err(|_| refused(format!("{attribute} is not a time")))
        })
        .transpose()
}

/// Reads and verifies a base64 `SAMLResponse`.
///
/// # Errors
///
/// [`ErrorKind::Unauthenticated`] for anything the module docs refuse.
pub fn read_response(
    encoded: &str,
    expected: &Expected<'_>,
) -> Result<Asserted, Error> {
    if encoded.len() > MAX_RESPONSE_BYTES {
        return Err(refused("the response is too large"));
    }
    let clean: String =
        encoded.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = STANDARD
        .decode(clean)
        .map_err(|_| refused("the response is not base64"))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| refused("the response is not UTF-8"))?;
    let doc = parse(&text)?;
    let response = doc.root_element();
    if !is(response, PROTOCOL_NS, "Response") {
        return Err(refused("this is not a SAML response"));
    }
    xmldsig::check_unique_ids(response)?;
    check_response(response, expected)?;
    if only_child(response, xmldsig::DSIG_NS, "Signature").is_some() {
        xmldsig::verify_enveloped(&text, response, &expected.idp.certificates)?;
    }
    if children(response, ASSERTION_NS, "EncryptedAssertion")
        .next()
        .is_some()
    {
        return Err(refused(
            "encrypted assertions are not supported; turn off assertion \
                encryption",
        ));
    }
    let assertion = only_child(response, ASSERTION_NS, "Assertion")
        .ok_or_else(|| {
            refused("the response must hold exactly one assertion")
        })?;
    xmldsig::verify_enveloped(&text, assertion, &expected.idp.certificates)?;
    // From here on, only the verified assertion is read.
    read_assertion(assertion, expected)
}

fn check_response(
    response: Node<'_, '_>,
    expected: &Expected<'_>,
) -> Result<(), Error> {
    if response.attribute("Version") != Some("2.0") {
        return Err(refused("the response is not SAML 2.0"));
    }
    if let Some(destination) = response.attribute("Destination")
        && destination != expected.sp.acs_url
    {
        return Err(refused("the response was sent to another address"));
    }
    if let Some(to) = response.attribute("InResponseTo")
        && to != expected.request_id
    {
        return Err(refused("the response answers another sign-in"));
    }
    if let Some(issuer) = only_child(response, ASSERTION_NS, "Issuer")
        && text_of(issuer)?.trim() != expected.idp.entity_id
    {
        return Err(refused("the response comes from another provider"));
    }
    let status = only_child(response, PROTOCOL_NS, "Status")
        .and_then(|s| only_child(s, PROTOCOL_NS, "StatusCode"))
        .and_then(|c| c.attribute("Value"));
    if status != Some(SUCCESS) {
        return Err(refused("the provider did not sign you in"));
    }
    Ok(())
}

fn read_assertion(
    assertion: Node<'_, '_>,
    expected: &Expected<'_>,
) -> Result<Asserted, Error> {
    let now = expected.now.unix_ms();
    let id = assertion
        .attribute("ID")
        .ok_or_else(|| refused("the assertion has no ID"))?
        .to_owned();
    let issuer = only_child(assertion, ASSERTION_NS, "Issuer")
        .map(text_of)
        .transpose()?
        .map(|t| t.trim().to_owned())
        .ok_or_else(|| refused("the assertion has no issuer"))?;
    if issuer != expected.idp.entity_id {
        return Err(refused("the assertion comes from another provider"));
    }
    let conditions = only_child(assertion, ASSERTION_NS, "Conditions")
        .ok_or_else(|| refused("the assertion has no conditions"))?;
    if time(conditions, "NotBefore")?
        .is_some_and(|t| t.unix_ms() > now + CLOCK_SKEW_MS)
    {
        return Err(refused(
            "the assertion is not valid yet; check the clocks",
        ));
    }
    let not_after = time(conditions, "NotOnOrAfter")?
        .ok_or_else(|| refused("the assertion has no expiry"))?;
    if not_after.unix_ms() + CLOCK_SKEW_MS <= now {
        return Err(refused("the assertion has expired"));
    }
    let mut restrictions =
        children(conditions, ASSERTION_NS, "AudienceRestriction").peekable();
    if restrictions.peek().is_none() {
        return Err(refused("the assertion names no audience"));
    }
    for restriction in restrictions {
        let mut ours = false;
        for audience in children(restriction, ASSERTION_NS, "Audience") {
            ours |= text_of(audience)?.trim() == expected.sp.entity_id;
        }
        if !ours {
            return Err(refused("the assertion is meant for another service"));
        }
    }
    let subject = only_child(assertion, ASSERTION_NS, "Subject")
        .ok_or_else(|| refused("the assertion has no subject"))?;
    let name_id_node = only_child(subject, ASSERTION_NS, "NameID")
        .ok_or_else(|| refused("the assertion has no NameID"))?;
    let name_id = text_of(name_id_node)?.trim().to_owned();
    if name_id.is_empty() {
        return Err(refused("the assertion has an empty NameID"));
    }
    let confirmed_until = confirmation(subject, expected)?;
    let (mut email, name) = attributes(assertion)?;
    if email.is_none()
        && (name_id_node.attribute("Format") == Some(EMAIL_FORMAT)
            || name_id.contains('@'))
    {
        email = Some(name_id.clone());
    }
    let until = not_after
        .unix_ms()
        .min(confirmed_until.unix_ms())
        .saturating_add(CLOCK_SKEW_MS)
        .min(now.saturating_add(MAX_REMEMBER_MS));
    let expires = Timestamp::from_unix_ms(until).unwrap_or(expected.now);
    let authn_instant = only_child(assertion, ASSERTION_NS, "AuthnStatement")
        .map(|s| time(s, "AuthnInstant"))
        .transpose()?
        .flatten();
    Ok(Asserted {
        authn_instant,
        id,
        issuer,
        name_id,
        email: email.map(|e| e.trim().to_lowercase()),
        name,
        expires,
    })
}

/// Checks the bearer confirmation and returns until when it holds.
fn confirmation(
    subject: Node<'_, '_>,
    expected: &Expected<'_>,
) -> Result<Timestamp, Error> {
    let now = expected.now.unix_ms();
    let mut problem = "the assertion has no bearer confirmation";
    for confirmation in children(subject, ASSERTION_NS, "SubjectConfirmation") {
        if confirmation.attribute("Method") != Some(BEARER) {
            continue;
        }
        let Some(data) =
            only_child(confirmation, ASSERTION_NS, "SubjectConfirmationData")
        else {
            continue;
        };
        if data.attribute("Recipient") != Some(expected.sp.acs_url.as_str()) {
            problem = "the assertion was meant for another address";
            continue;
        }
        if data.attribute("InResponseTo") != Some(expected.request_id) {
            problem = "the assertion answers another sign-in";
            continue;
        }
        if time(data, "NotBefore")?
            .is_some_and(|t| t.unix_ms() > now + CLOCK_SKEW_MS)
        {
            problem = "the assertion is not valid yet; check the clocks";
            continue;
        }
        match time(data, "NotOnOrAfter")? {
            Some(until) if until.unix_ms() + CLOCK_SKEW_MS > now => {
                return Ok(until);
            }
            Some(_) => problem = "the assertion has expired",
            None => problem = "the assertion's confirmation has no expiry",
        }
    }
    Err(refused(problem))
}

/// The email and display name from the attribute statements.
fn attributes(
    assertion: Node<'_, '_>,
) -> Result<(Option<String>, Option<String>), Error> {
    let mut email = None;
    let mut name = None;
    for statement in children(assertion, ASSERTION_NS, "AttributeStatement") {
        for attribute in children(statement, ASSERTION_NS, "Attribute") {
            let key = attribute
                .attribute("Name")
                .unwrap_or_default()
                .to_lowercase();
            let Some(value) =
                children(attribute, ASSERTION_NS, "AttributeValue").next()
            else {
                continue;
            };
            let value = text_of(value)?.trim().to_owned();
            if value.is_empty() {
                continue;
            }
            if email.is_none() && EMAIL_ATTRIBUTES.contains(&key.as_str()) {
                email = Some(value);
            } else if name.is_none() && NAME_ATTRIBUTES.contains(&key.as_str())
            {
                name = Some(value);
            }
        }
    }
    Ok((email, name))
}

/// Reads a provider's metadata: entity ID, HTTP-Redirect sign-in URL and
/// signing certificates.
///
/// # Errors
///
/// [`ErrorKind::InvalidInput`] when the document is not identity
/// provider metadata or lacks one of those.
pub fn parse_metadata(bytes: &[u8]) -> Result<Metadata, Error> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| invalid("the metadata is not UTF-8"))?;
    let doc = parse(text).map_err(|e| invalid(e.message().to_owned()))?;
    let root = doc.root_element();
    let entity = if is(root, METADATA_NS, "EntityDescriptor") {
        Some(root)
    } else if is(root, METADATA_NS, "EntitiesDescriptor") {
        let mut found =
            children(root, METADATA_NS, "EntityDescriptor").filter(|e| {
                only_child(*e, METADATA_NS, "IDPSSODescriptor").is_some()
            });
        let first = found.next();
        if found.next().is_some() {
            return Err(invalid(
                "the metadata describes several providers; use one provider's \
                    URL",
            ));
        }
        first
    } else {
        None
    }
    .ok_or_else(|| invalid("this is not SAML metadata"))?;
    let entity_id = entity
        .attribute("entityID")
        .ok_or_else(|| invalid("the metadata has no entity ID"))?
        .to_owned();
    let idp = only_child(entity, METADATA_NS, "IDPSSODescriptor").ok_or_else(
        || invalid("the metadata does not describe an identity provider"),
    )?;
    let sso_url = children(idp, METADATA_NS, "SingleSignOnService")
        .find(|s| s.attribute("Binding") == Some(HTTP_REDIRECT))
        .and_then(|s| s.attribute("Location"))
        .ok_or_else(|| invalid("the provider offers no HTTP-Redirect sign-in"))?
        .to_owned();
    let mut certificates = Vec::new();
    for key in children(idp, METADATA_NS, "KeyDescriptor") {
        if key.attribute("use").is_some_and(|u| u != "signing") {
            continue;
        }
        for cert in key
            .descendants()
            .filter(|n| is(*n, xmldsig::DSIG_NS, "X509Certificate"))
        {
            let text =
                text_of(cert).map_err(|e| invalid(e.message().to_owned()))?;
            certificates.extend(Certificate::parse_all(&text)?);
        }
    }
    if certificates.is_empty() {
        return Err(invalid("the metadata has no signing certificate"));
    }
    Ok(Metadata {
        entity_id,
        sso_url,
        certificates,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests fail loudly")]

    use std::io::Read as _;

    use flate2::read::DeflateDecoder;

    use super::*;

    fn sp() -> ServiceProvider {
        ServiceProvider {
            entity_id: "https://vellum.northstar.dev/saml/metadata".into(),
            acs_url: "https://vellum.northstar.dev/saml/acs".into(),
        }
    }

    #[test]
    fn the_request_is_deflated_into_the_sign_in_url() {
        let idp = IdentityProvider {
            entity_id: "https://idp.northstar.dev".into(),
            sso_url: "https://idp.northstar.dev/sso?tenant=1".into(),
            certificates: Vec::new(),
        };
        let url = authn_request_url(
            &sp(),
            &idp,
            "_abc",
            "_relay",
            true,
            Timestamp::now(),
        )
        .unwrap();
        let url = Url::parse(&url).unwrap();
        let pairs: Vec<(String, String)> =
            url.query_pairs().into_owned().collect();
        assert_eq!(pairs[0], ("tenant".into(), "1".into()));
        assert_eq!(pairs[2], ("RelayState".into(), "_relay".into()));
        let deflated = STANDARD.decode(&pairs[1].1).unwrap();
        let mut xml = String::new();
        let _ = DeflateDecoder::new(deflated.as_slice())
            .read_to_string(&mut xml)
            .unwrap();
        assert!(xml.contains("ID=\"_abc\""));
        assert!(xml.contains("ForceAuthn=\"true\""));
        assert!(xml.contains(
            "AssertionConsumerServiceURL=\"https://vellum.northsta\
                r.dev/saml/acs\""
        ));
        assert!(xml.contains(
            "<saml:Issuer>https://vellum.northstar.dev/saml/metada\
                ta</saml:Issuer>"
        ));
        let doc = Document::parse(&xml).unwrap();
        assert!(is(doc.root_element(), PROTOCOL_NS, "AuthnRequest"));
    }

    #[test]
    fn ids_are_fresh_and_xml_names() {
        let a = new_id().unwrap();
        assert_eq!(a.len(), 41);
        assert!(a.starts_with('_'));
        assert_ne!(a, new_id().unwrap());
    }

    #[test]
    fn metadata_with_a_dtd_is_refused() {
        let xml = concat!(
            r#"<?xml version="1.0"?><!DOCTYPE x [<!ENTITY a "#,
            r#""aaaa">]><x>&a;</x>"#,
        );
        assert_eq!(
            parse_metadata(xml.as_bytes()).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }

    #[test]
    fn sp_metadata_parses() {
        let xml = sp_metadata(&sp());
        let doc = Document::parse(&xml).unwrap();
        assert_eq!(
            doc.root_element().attribute("entityID"),
            Some("https://vellum.northstar.dev/saml/metadata")
        );
    }
}
