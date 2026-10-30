// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! SAML 2.0 sign-in (issue #523, ADR 0069 D293), including the red-team
//! cases: signature wrapping, XML entity expansion, replayed assertions,
//! and tampering.

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test helpers fail loudly"
)]

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use sha2::Digest as _;
use vellum_identity::saml::{Expected, IdentityProvider, ServiceProvider, read_response};
use vellum_identity::xmldsig::Certificate;
use vellum_kernel::{ErrorKind, Timestamp};

const IDP: &str = "https://idp.northstar.dev";

fn sp() -> ServiceProvider {
    ServiceProvider {
        entity_id: "https://vellum.northstar.dev/saml/metadata".into(),
        acs_url: "https://vellum.northstar.dev/saml/acs".into(),
    }
}

fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/saml/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(path).unwrap()
}

fn at(text: &str) -> Timestamp {
    text.parse().unwrap()
}

fn python_idp(tag: &str) -> IdentityProvider {
    IdentityProvider {
        entity_id: IDP.into(),
        sso_url: format!("{IDP}/sso"),
        certificates: Certificate::parse_all(&fixture(&format!("{tag}-cert.pem"))).unwrap(),
    }
}

fn read_fixture(
    xml: &str,
    idp: &IdentityProvider,
    now: &str,
) -> Result<vellum_identity::saml::Asserted, vellum_kernel::Error> {
    let sp = sp();
    read_response(
        &STANDARD.encode(xml),
        &Expected {
            sp: &sp,
            idp,
            request_id: "_python-request",
            now: at(now),
        },
    )
}

#[test]
fn responses_signed_by_another_implementation_verify() {
    for tag in ["rsa", "ecdsa"] {
        let idp = python_idp(tag);
        let xml = fixture(&format!("{tag}-response.xml"));
        let asserted = read_fixture(&xml, &idp, "2026-10-30T12:01:00Z").unwrap();
        assert_eq!(asserted.id, format!("_assert-{tag}"));
        assert_eq!(asserted.issuer, IDP);
        assert_eq!(asserted.name_id, "00u1priya");
        assert_eq!(asserted.email.as_deref(), Some("priya@northstar.dev"));
        assert_eq!(asserted.name.as_deref(), Some("Priya Raman & co"));
        assert_eq!(asserted.expires, at("2026-10-30T12:07:00Z"));
    }
}

#[test]
fn a_changed_or_mis_keyed_response_is_refused() {
    let idp = python_idp("rsa");
    let xml = fixture("rsa-response.xml");
    let tampered = xml.replace("Priya@Northstar.dev", "eve@northstar.dev");
    let err = read_fixture(&tampered, &idp, "2026-10-30T12:01:00Z").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unauthenticated);
    assert!(err.message().contains("changed after signing"), "{err}");
    // The ECDSA provider's certificate does not verify the RSA response.
    let err = read_fixture(&xml, &python_idp("ecdsa"), "2026-10-30T12:01:00Z").unwrap_err();
    assert!(err.message().contains("certificate"), "{err}");
    // Too late, and for another sign-in.
    let err = read_fixture(&xml, &idp, "2026-10-30T12:10:00Z").unwrap_err();
    assert!(err.message().contains("expired"), "{err}");
}

// --- A test identity provider, signing with Vellum's canonicalization ---

const ACS: &str = "https://vellum.northstar.dev/saml/acs";
const SP_ID: &str = "https://vellum.northstar.dev/saml/metadata";
const REQUEST: &str = "_request-1";
const NOW: &str = "2026-10-30T12:01:00Z";

/// What goes into a test response.
#[derive(Clone)]
struct Claims {
    assertion_id: String,
    issuer: String,
    audience: String,
    recipient: String,
    in_response_to: String,
    not_on_or_after: String,
    not_before: String,
    name_id: String,
    email: String,
    status: String,
}

impl Default for Claims {
    fn default() -> Self {
        Self {
            assertion_id: "_a1".into(),
            issuer: IDP.into(),
            audience: SP_ID.into(),
            recipient: ACS.into(),
            in_response_to: REQUEST.into(),
            not_on_or_after: "2026-10-30T12:05:00Z".into(),
            not_before: "2026-10-30T11:59:00Z".into(),
            name_id: "00u1priya".into(),
            email: "priya@northstar.dev".into(),
            status: "urn:oasis:names:tc:SAML:2.0:status:Success".into(),
        }
    }
}

fn assertion_xml(c: &Claims) -> String {
    format!(
        r#"<saml:Assertion xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="{id}" Version="2.0" IssueInstant="2026-10-30T12:00:00Z"><saml:Issuer>{issuer}</saml:Issuer><saml:Subject><saml:NameID>{name_id}</saml:NameID><saml:SubjectConfirmation Method="urn:oasis:names:tc:SAML:2.0:cm:bearer"><saml:SubjectConfirmationData InResponseTo="{irt}" NotOnOrAfter="{nooa}" Recipient="{recipient}"/></saml:SubjectConfirmation></saml:Subject><saml:Conditions NotBefore="{nb}" NotOnOrAfter="{nooa}"><saml:AudienceRestriction><saml:Audience>{audience}</saml:Audience></saml:AudienceRestriction></saml:Conditions><saml:AttributeStatement><saml:Attribute Name="email"><saml:AttributeValue>{email}</saml:AttributeValue></saml:Attribute></saml:AttributeStatement></saml:Assertion>"#,
        id = c.assertion_id,
        issuer = c.issuer,
        name_id = c.name_id,
        irt = c.in_response_to,
        nooa = c.not_on_or_after,
        recipient = c.recipient,
        nb = c.not_before,
        audience = c.audience,
        email = c.email,
    )
}

fn response_xml(status: &str, assertions: &str) -> String {
    format!(
        r#"<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" ID="_r1" Version="2.0" IssueInstant="2026-10-30T12:00:00Z" Destination="{ACS}" InResponseTo="{REQUEST}"><samlp:Status><samlp:StatusCode Value="{status}"/></samlp:Status>{assertions}</samlp:Response>"#
    )
}

struct TestIdp {
    signer: aws_lc_rs::signature::EcdsaKeyPair,
    certificate: String,
}

impl TestIdp {
    fn new() -> Self {
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["idp.northstar.dev".to_owned()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        let signer = aws_lc_rs::signature::EcdsaKeyPair::from_pkcs8(
            &aws_lc_rs::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &key.serialize_der(),
        )
        .unwrap();
        Self {
            signer,
            certificate: cert.pem(),
        }
    }

    fn provider(&self) -> IdentityProvider {
        IdentityProvider {
            entity_id: IDP.into(),
            sso_url: format!("{IDP}/sso"),
            certificates: Certificate::parse_all(&self.certificate).unwrap(),
        }
    }

    /// Signs the element with `id` in `xml` (enveloped, after its Issuer
    /// or first child) and returns the document.
    fn sign(&self, xml: &str, id: &str, method: &str) -> String {
        use vellum_identity::xmldsig::exc_c14n;
        let doc = roxmltree::Document::parse(xml).unwrap();
        let target = doc
            .descendants()
            .find(|n| n.attribute("ID") == Some(id))
            .unwrap();
        let canonical = exc_c14n(xml, target, None, &[]).unwrap();
        let digest = STANDARD.encode(sha2::Sha256::digest(canonical.as_bytes()));
        let signature = format!(
            r##"<ds:Signature xmlns:ds="http://www.w3.org/2000/09/xmldsig#"><ds:SignedInfo><ds:CanonicalizationMethod Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"/><ds:SignatureMethod Algorithm="{method}"/><ds:Reference URI="#{id}"><ds:Transforms><ds:Transform Algorithm="http://www.w3.org/2000/09/xmldsig#enveloped-signature"/><ds:Transform Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"/></ds:Transforms><ds:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><ds:DigestValue>{digest}</ds:DigestValue></ds:Reference></ds:SignedInfo><ds:SignatureValue>VALUE</ds:SignatureValue></ds:Signature>"##
        );
        // Insert as the second child: after the Issuer or Status.
        let first = target.first_element_child().unwrap();
        let at = first.range().end;
        let signed = format!("{}{}{}", &xml[..at], signature, &xml[at..]);
        let doc = roxmltree::Document::parse(&signed).unwrap();
        let info = doc
            .descendants()
            .find(|n| {
                n.tag_name().name() == "SignedInfo"
                    && n.parent()
                        .and_then(|p| p.parent())
                        .and_then(|p| p.attribute("ID"))
                        == Some(id)
            })
            .unwrap();
        let canonical = exc_c14n(&signed, info, None, &[]).unwrap();
        let rng = aws_lc_rs::rand::SystemRandom::new();
        let value = self.signer.sign(&rng, canonical.as_bytes()).unwrap();
        let value = STANDARD.encode(value.as_ref());
        // Fill in this signature's value (the last placeholder added).
        let at = signed.find("VALUE").unwrap();
        format!("{}{}{}", &signed[..at], value, &signed[at + 5..])
    }

    fn signed_assertion(&self, claims: &Claims) -> String {
        self.sign(&assertion_xml(claims), &claims.assertion_id, ECDSA)
    }

    fn response(&self, claims: &Claims) -> String {
        response_xml(&claims.status, &self.signed_assertion(claims))
    }
}

const ECDSA: &str = "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256";

fn check(
    idp: &TestIdp,
    xml: &str,
) -> Result<vellum_identity::saml::Asserted, vellum_kernel::Error> {
    let sp = sp();
    let provider = idp.provider();
    read_response(
        &STANDARD.encode(xml),
        &Expected {
            sp: &sp,
            idp: &provider,
            request_id: REQUEST,
            now: at(NOW),
        },
    )
}

fn refused_with(idp: &TestIdp, xml: &str, words: &str) {
    let err = check(idp, xml).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unauthenticated, "{err}");
    assert!(
        err.message().contains(words),
        "expected “{words}”, got “{err}”"
    );
}

#[test]
fn a_well_formed_signed_response_is_accepted() {
    let idp = TestIdp::new();
    let asserted = check(&idp, &idp.response(&Claims::default())).unwrap();
    assert_eq!(asserted.email.as_deref(), Some("priya@northstar.dev"));
    assert_eq!(asserted.name_id, "00u1priya");
}

#[test]
fn every_condition_is_checked() {
    let idp = TestIdp::new();
    let cases: [(&str, Claims); 7] = [
        (
            "another service",
            Claims {
                audience: "https://evil.example/sp".into(),
                ..Claims::default()
            },
        ),
        (
            "another address",
            Claims {
                recipient: "https://evil.example/acs".into(),
                ..Claims::default()
            },
        ),
        (
            "another sign-in",
            Claims {
                in_response_to: "_other".into(),
                ..Claims::default()
            },
        ),
        (
            "another provider",
            Claims {
                issuer: "https://evil.example".into(),
                ..Claims::default()
            },
        ),
        (
            "expired",
            Claims {
                not_on_or_after: "2026-10-30T11:58:00Z".into(),
                ..Claims::default()
            },
        ),
        (
            "not valid yet",
            Claims {
                not_before: "2026-10-30T12:10:00Z".into(),
                ..Claims::default()
            },
        ),
        (
            "did not sign you in",
            Claims {
                status: "urn:oasis:names:tc:SAML:2.0:status:Requester".into(),
                ..Claims::default()
            },
        ),
    ];
    for (words, claims) in cases {
        refused_with(&idp, &idp.response(&claims), words);
    }
}

#[test]
fn unsigned_or_wrongly_signed_assertions_are_refused() {
    let idp = TestIdp::new();
    let claims = Claims::default();
    refused_with(
        &idp,
        &response_xml(&claims.status, &assertion_xml(&claims)),
        "not signed",
    );
    // Signed by someone else.
    let other = TestIdp::new();
    refused_with(&idp, &other.response(&claims), "certificate");
    // SHA-1 and unknown algorithms are refused before any key is tried.
    let sha1 = idp.sign(
        &assertion_xml(&claims),
        "_a1",
        "http://www.w3.org/2000/09/xmldsig#rsa-sha1",
    );
    refused_with(
        &idp,
        &response_xml(&claims.status, &sha1),
        "RSA-SHA256 or ECDSA-SHA256",
    );
    // Encrypted assertions are not read.
    let encrypted = response_xml(
        &claims.status,
        r#"<saml:EncryptedAssertion xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion"/>"#,
    );
    refused_with(&idp, &encrypted, "encrypted");
}

#[test]
fn signature_wrapping_is_refused() {
    let idp = TestIdp::new();
    let genuine = idp.signed_assertion(&Claims::default());
    let evil = Claims {
        assertion_id: "_evil".into(),
        name_id: "admin".into(),
        email: "admin@northstar.dev".into(),
        ..Claims::default()
    };
    // 1. A forged assertion next to the signed one.
    let both = response_xml(&evil.status, &format!("{}{genuine}", assertion_xml(&evil)));
    refused_with(&idp, &both, "exactly one assertion");
    // 2. The signed assertion hidden inside the forged one.
    let forged = assertion_xml(&evil).replace(
        "<saml:Subject>",
        &format!("<saml:Advice>{genuine}</saml:Advice><saml:Subject>"),
    );
    refused_with(&idp, &response_xml(&evil.status, &forged), "not signed");
    // 3. The genuine signature moved onto the forged assertion.
    let signature = &genuine
        [genuine.find("<ds:Signature").unwrap()..genuine.find("</ds:Signature>").unwrap() + 15];
    let moved =
        assertion_xml(&evil).replacen("</saml:Issuer>", &format!("</saml:Issuer>{signature}"), 1);
    refused_with(&idp, &response_xml(&evil.status, &moved), "does not cover");
    // 4. ...with the forged assertion taking the genuine ID, the genuine
    //    one tucked into the response's extensions.
    let same_id = moved.replace("ID=\"_evil\"", "ID=\"_a1\"");
    let hidden = format!("<samlp:Extensions>{genuine}</samlp:Extensions>{same_id}");
    refused_with(
        &idp,
        &response_xml(&evil.status, &hidden),
        "repeats an element ID",
    );
    // 5. Same ID, no copy of the genuine assertion: the digest no longer
    //    matches.
    refused_with(
        &idp,
        &response_xml(&evil.status, &same_id),
        "changed after signing",
    );
}

#[test]
fn entity_expansion_and_doctypes_are_refused() {
    let idp = TestIdp::new();
    let bomb = r#"<?xml version="1.0"?><!DOCTYPE r [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;"><!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;">]><samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol">&c;</samlp:Response>"#;
    refused_with(&idp, bomb, "cannot be read");
    let external =
        r#"<?xml version="1.0"?><!DOCTYPE r [<!ENTITY x SYSTEM "file:///etc/passwd">]><r>&x;</r>"#;
    refused_with(&idp, external, "cannot be read");
}

#[test]
fn a_comment_cannot_shorten_a_signed_name() {
    let idp = TestIdp::new();
    let claims = Claims {
        name_id: "admin@northstar.dev.evil.example".into(),
        email: "x@evil.example".into(),
        ..Claims::default()
    };
    let signed = idp.response(&claims);
    // Comments are outside the signature, so inserting one keeps it valid;
    // the value must still not read as "admin@northstar.dev".
    let commented = signed.replace(
        "admin@northstar.dev.evil.example",
        "admin@northstar.dev<!---->.evil.example",
    );
    refused_with(&idp, &commented, "more than text");
}

// --- Signing in through Admin → Sign-in, and over HTTP ---

mod flows {
    use std::io::Read as _;
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use http::{Request, StatusCode, header};
    use http_body_util::BodyExt as _;
    use tower::ServiceExt as _;
    use url::Url;
    use vellum_identity::providers::{ProviderInput, ProviderKind};
    use vellum_identity::{Identity, IdentityConfig, Store, http as identity_http};

    use super::*;

    fn identity() -> Identity {
        let url = Url::parse("https://vellum.northstar.dev").unwrap();
        Identity::new(IdentityConfig::new(url), Store::memory())
    }

    fn saml_input(idp: &TestIdp) -> ProviderInput {
        ProviderInput {
            name: "Northstar SAML".into(),
            enabled: true,
            kind: ProviderKind::Saml,
            issuer: String::new(),
            client_id: String::new(),
            client_secret: None,
            allowed_domains: vec!["northstar.dev".into()],
            auto_create: true,
            entity_id: IDP.into(),
            sso_url: format!("{IDP}/sso"),
            certificate: idp.certificate.clone(),
            metadata_url: None,
        }
    }

    /// The request ID inside a sign-in URL.
    fn request_id(url: &str) -> (String, String) {
        let url = Url::parse(url).unwrap();
        let pairs: std::collections::HashMap<String, String> =
            url.query_pairs().into_owned().collect();
        let deflated = STANDARD.decode(&pairs["SAMLRequest"]).unwrap();
        let mut xml = String::new();
        let _ = flate2::read::DeflateDecoder::new(deflated.as_slice())
            .read_to_string(&mut xml)
            .unwrap();
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let id = doc.root_element().attribute("ID").unwrap().to_owned();
        (id, pairs["RelayState"].clone())
    }

    fn live(request: &str, assertion_id: &str, email: &str) -> Claims {
        let now = Timestamp::now().unix_ms();
        let time = |offset: i64| Timestamp::from_unix_ms(now + offset).unwrap().to_string();
        Claims {
            assertion_id: assertion_id.into(),
            in_response_to: request.into(),
            not_before: time(-60_000),
            not_on_or_after: time(300_000),
            email: email.into(),
            name_id: format!("id-{email}"),
            ..Claims::default()
        }
    }

    fn post(idp: &TestIdp, claims: &Claims) -> String {
        let xml = idp.response(claims).replace(
            &format!("InResponseTo=\"{REQUEST}\""),
            &format!("InResponseTo=\"{}\"", claims.in_response_to),
        );
        STANDARD.encode(xml)
    }

    #[test]
    fn a_saml_provider_signs_people_in_once_per_response() {
        let id = identity();
        let _ = id
            .setup(
                "priya@northstar.dev",
                "Priya",
                "correct horse battery staple",
            )
            .unwrap();
        let idp = TestIdp::new();
        let provider = id.add_provider(&saml_input(&idp)).unwrap();
        assert_eq!(provider.kind, "saml");
        assert_eq!(provider.certificate_fingerprints.len(), 1);
        assert!(id.is_saml_provider(&provider.id));

        let started = id.saml_start(&provider.id, Some("/data"), false).unwrap();
        let (request, relay) = request_id(&started.url);
        assert_eq!(relay, started.relay_state);
        let claims = live(&request, "_once", "sam@northstar.dev");
        let response = post(&idp, &claims);

        // Another browser (no or another cookie) cannot finish it.
        assert_eq!(
            id.saml_finish(&relay, None, &response, None)
                .unwrap_err()
                .kind(),
            ErrorKind::Unauthenticated
        );
        // The first try above did not use it up: the same browser can.
        let (session, next) = id
            .saml_finish(&relay, Some(&relay), &response, None)
            .unwrap();
        assert_eq!(next, "/data");
        let sam = id.user(session.session.user).unwrap();
        assert_eq!(sam.email, "sam@northstar.dev");
        assert!(!sam.admin);

        // Posting it again fails: the sign-in is finished.
        let err = id
            .saml_finish(&relay, Some(&relay), &response, None)
            .unwrap_err();
        assert!(err.message().contains("already used"), "{err}");

        // The same assertion answering a new sign-in is a replay.
        let again = id.saml_start(&provider.id, None, false).unwrap();
        let (request2, relay2) = request_id(&again.url);
        let replayed = post(&idp, &live(&request2, "_once", "sam@northstar.dev"));
        let err = id
            .saml_finish(&relay2, Some(&relay2), &replayed, None)
            .unwrap_err();
        assert!(err.message().contains("already used"), "{err}");
    }

    #[test]
    fn domains_and_the_provider_switch_are_respected() {
        let id = identity();
        let _ = id
            .setup(
                "priya@northstar.dev",
                "Priya",
                "correct horse battery staple",
            )
            .unwrap();
        let idp = TestIdp::new();
        let provider = id.add_provider(&saml_input(&idp)).unwrap();
        let started = id.saml_start(&provider.id, None, false).unwrap();
        let (request, relay) = request_id(&started.url);
        let outsider = post(&idp, &live(&request, "_out", "mallory@evil.example"));
        let err = id
            .saml_finish(&relay, Some(&relay), &outsider, None)
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::PermissionDenied);

        let mut off = saml_input(&idp);
        off.enabled = false;
        off.certificate = String::new();
        let changed = id.update_provider(&provider.id, &off).unwrap();
        assert_eq!(
            changed.certificate_fingerprints.len(),
            1,
            "the certificate is kept"
        );
        assert_eq!(
            id.saml_start(&provider.id, None, false).unwrap_err().kind(),
            ErrorKind::NotFound
        );
        // A provider keeps its kind.
        let mut oidc = off;
        oidc.kind = ProviderKind::Oidc;
        oidc.issuer = "https://id.northstar.dev".into();
        oidc.client_id = "vellum".into();
        assert_eq!(
            id.update_provider(&provider.id, &oidc).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }

    async fn send(
        router: &Router,
        request: Request<Body>,
    ) -> (StatusCode, http::HeaderMap, String) {
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        (status, headers, String::from_utf8_lossy(&body).into_owned())
    }

    fn cookies(headers: &http::HeaderMap) -> Vec<String> {
        headers
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|v| v.to_str().unwrap().to_owned())
            .collect()
    }

    #[tokio::test]
    async fn a_browser_signs_in_through_the_test_idp() {
        let id = identity();
        let _ = id
            .setup(
                "priya@northstar.dev",
                "Priya",
                "correct horse battery staple",
            )
            .unwrap();
        let idp = TestIdp::new();
        let provider = id.add_provider(&saml_input(&idp)).unwrap();
        let id = Arc::new(id);
        let router = Router::new()
            .nest("/auth", identity_http::browser_router(Arc::clone(&id)))
            .nest("/saml", identity_http::saml_router(Arc::clone(&id)));

        // The sign-in button goes to the provider and sets the relay cookie.
        let start = Request::get(format!("{}&next=%2Fdata", provider.start_url))
            .body(Body::empty())
            .unwrap();
        let (status, headers, _) = send(&router, start).await;
        assert!(status.is_redirection(), "{status}");
        let location = headers[header::LOCATION].to_str().unwrap().to_owned();
        assert!(location.starts_with(&format!("{IDP}/sso?SAMLRequest=")));
        let relay_cookie = cookies(&headers)
            .into_iter()
            .find(|c| c.starts_with("__Host-vellum_saml="))
            .unwrap();
        assert!(
            relay_cookie.contains("SameSite=None; Secure"),
            "{relay_cookie}"
        );
        let (request, relay) = request_id(&location);

        // The provider posts the signed response back.
        let response = post(&idp, &live(&request, "_http", "priya@northstar.dev"));
        let form = serde_urlencoded::to_string([
            ("SAMLResponse", response.as_str()),
            ("RelayState", relay.as_str()),
        ])
        .unwrap();
        let acs = |cookie: Option<&str>| {
            let mut request = Request::post("/saml/acs")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
            if let Some(cookie) = cookie {
                request = request.header(header::COOKIE, cookie);
            }
            request.body(Body::from(form.clone())).unwrap()
        };
        // Without the cookie (a response pushed into another browser).
        let (status, _, page) = send(&router, acs(None)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(page.contains("another browser"), "{page}");
        let cookie = format!("__Host-vellum_saml={relay}");
        let (status, headers, _) = send(&router, acs(Some(&cookie))).await;
        assert!(status.is_redirection(), "{status}");
        assert_eq!(headers[header::LOCATION], "/data");
        let set = cookies(&headers);
        assert!(
            set.iter()
                .any(|c| c.starts_with("__Host-vellum_session=") && !c.contains("Max-Age=0")),
            "{set:?}"
        );
        assert!(
            set.iter().any(|c| c.starts_with("__Host-vellum_saml=;")),
            "{set:?}"
        );

        // Vellum's metadata is served for the provider's admin.
        let (status, headers, xml) = send(
            &router,
            Request::get("/saml/metadata").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers[header::CONTENT_TYPE],
            "application/samlmetadata+xml"
        );
        assert!(xml.contains("Location=\"https://vellum.northstar.dev/saml/acs\""));
    }
}
