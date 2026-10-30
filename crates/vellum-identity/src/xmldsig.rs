// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! XML signatures, only as far as SAML needs them (ADR 0069, D293).
//!
//! - One enveloped signature, a direct child of the element it signs,
//!   whose single reference names that element's `ID`. Anything else is
//!   refused, which with unique IDs ([`check_unique_ids`]) and reading
//!   claims only from the verified element blocks signature wrapping.
//! - Exclusive canonicalization without comments, SHA-256 digests, and
//!   RSA (PKCS#1 v1.5) or ECDSA P-256 signatures. SHA-1 is refused.
//! - The key is always one of the provider's pinned certificates; a key
//!   inside the document is ignored.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Write as _;

use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED, RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey,
};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use roxmltree::{Node, NodeId, NodeType};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use vellum_kernel::{Error, ErrorKind};
use x509_parser::prelude::{FromDer as _, X509Certificate};

/// The XML signature namespace.
pub const DSIG_NS: &str = "http://www.w3.org/2000/09/xmldsig#";
/// Exclusive XML canonicalization, without comments.
pub const EXC_C14N: &str = "http://www.w3.org/2001/10/xml-exc-c14n#";
/// The enveloped-signature transform.
pub const ENVELOPED: &str =
    "http://www.w3.org/2000/09/xmldsig#enveloped-signature";
/// SHA-256 digests.
pub const SHA256: &str = "http://www.w3.org/2001/04/xmlenc#sha256";
/// RSA PKCS#1 v1.5 with SHA-256.
pub const RSA_SHA256: &str =
    "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256";
/// ECDSA with SHA-256.
pub const ECDSA_SHA256: &str =
    "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256";

/// The deepest element nesting canonicalized.
const MAX_DEPTH: usize = 64;
/// OID of `rsaEncryption`.
const OID_RSA: &str = "1.2.840.113549.1.1.1";
/// OID of `id-ecPublicKey`.
const OID_EC: &str = "1.2.840.10045.2.1";
/// OID of the P-256 curve.
const OID_P256: &str = "1.2.840.10045.3.1.7";

fn refused(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::Unauthenticated, message.into())
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message.into())
}

/// A provider's signing certificate, pinned in its settings.
#[derive(Clone, PartialEq, Eq)]
pub struct Certificate {
    der: Vec<u8>,
    key: Key,
}

#[derive(Clone, PartialEq, Eq)]
enum Key {
    /// A PKCS#1 `RSAPublicKey`.
    Rsa(Vec<u8>),
    /// An uncompressed P-256 point.
    P256(Vec<u8>),
}

impl std::fmt::Debug for Certificate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Certificate")
            .field(&self.fingerprint())
            .finish()
    }
}

impl Certificate {
    /// Reads a DER certificate with an RSA or P-256 key.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] for anything else.
    pub fn from_der(der: &[u8]) -> Result<Self, Error> {
        let (_, cert) = X509Certificate::from_der(der).map_err(|e| {
            invalid(format!("the certificate cannot be read: {e}"))
        })?;
        let spki = cert.public_key();
        let bits = spki.subject_public_key.data.to_vec();
        let key = match spki.algorithm.algorithm.to_id_string().as_str() {
            OID_RSA => Key::Rsa(bits),
            OID_EC => {
                let curve = spki
                    .algorithm
                    .parameters
                    .as_ref()
                    .and_then(|p| p.as_oid().ok())
                    .map(|o| o.to_id_string());
                if curve.as_deref() != Some(OID_P256) {
                    return Err(invalid(
                        "only P-256 elliptic-curve keys are supported",
                    ));
                }
                Key::P256(bits)
            }
            _ => {
                return Err(invalid(
                    "the certificate needs an RSA or P-256 key",
                ));
            }
        };
        Ok(Self {
            der: der.to_vec(),
            key,
        })
    }

    /// Reads every certificate in PEM text, or one in bare base64 (as
    /// metadata holds them).
    ///
    /// # Errors
    ///
    /// [`ErrorKind::InvalidInput`] when there is none or one is unreadable.
    pub fn parse_all(text: &str) -> Result<Vec<Self>, Error> {
        let blocks: Vec<String> = if text.contains("-----BEGIN") {
            text.split("-----BEGIN CERTIFICATE-----")
                .skip(1)
                .map(|block| {
                    block
                        .split("-----END CERTIFICATE-----")
                        .next()
                        .unwrap_or_default()
                        .to_owned()
                })
                .collect()
        } else {
            vec![text.to_owned()]
        };
        let certs = blocks
            .iter()
            .map(|b| {
                let clean: String =
                    b.chars().filter(|c| !c.is_whitespace()).collect();
                let der = STANDARD.decode(clean).map_err(|_| {
                    invalid("a certificate is not valid base64")
                })?;
                Self::from_der(&der)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if certs.is_empty() {
            return Err(invalid("paste the provider's signing certificate"));
        }
        Ok(certs)
    }

    /// The DER bytes, base64.
    #[must_use]
    pub fn to_base64(&self) -> String {
        STANDARD.encode(&self.der)
    }

    /// The certificate as PEM.
    #[must_use]
    pub fn to_pem(&self) -> String {
        let body = self.to_base64();
        let mut pem = String::from("-----BEGIN CERTIFICATE-----\n");
        for line in body.as_bytes().chunks(64) {
            pem.push_str(&String::from_utf8_lossy(line));
            pem.push('\n');
        }
        pem.push_str("-----END CERTIFICATE-----\n");
        pem
    }

    /// The SHA-256 fingerprint, as colon-separated hex.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        Sha256::digest(&self.der)
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    }

    fn verify(
        &self,
        algorithm: &str,
        message: &[u8],
        signature: &[u8],
    ) -> bool {
        match (&self.key, algorithm) {
            (Key::Rsa(key), RSA_SHA256) => {
                UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, key)
                    .verify(message, signature)
                    .is_ok()
            }
            (Key::P256(key), ECDSA_SHA256) => {
                UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, key)
                    .verify(message, signature)
                    .is_ok()
            }
            _ => false,
        }
    }
}

/// The element `node`'s child in `ns` named `name`, when there is exactly
/// one.
pub(crate) fn only_child<'a, 'i>(
    node: Node<'a, 'i>,
    ns: &str,
    name: &str,
) -> Option<Node<'a, 'i>> {
    let mut found = None;
    for child in node.children() {
        if child.is_element()
            && child.tag_name().namespace() == Some(ns)
            && child.tag_name().name() == name
        {
            if found.is_some() {
                return None;
            }
            found = Some(child);
        }
    }
    found
}

/// The element `node`'s children in `ns` named `name`.
pub(crate) fn children<'a, 'i: 'a>(
    node: Node<'a, 'i>,
    ns: &'a str,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    node.children().filter(move |c| {
        c.is_element()
            && c.tag_name().namespace() == Some(ns)
            && c.tag_name().name() == name
    })
}

/// Refuses a document where two elements share an `ID`, so a reference
/// can only mean one element.
///
/// # Errors
///
/// [`ErrorKind::Unauthenticated`] on a repeated ID.
pub fn check_unique_ids(root: Node<'_, '_>) -> Result<(), Error> {
    let mut seen = HashSet::new();
    for node in root.descendants().filter(Node::is_element) {
        for attr in node.attributes() {
            if matches!(attr.name(), "ID" | "Id" | "id")
                && !seen.insert(attr.value())
            {
                return Err(refused("the response repeats an element ID"));
            }
        }
    }
    Ok(())
}

/// Checks the enveloped signature that is a direct child of `signed`
/// against `certificates`.
///
/// # Errors
///
/// [`ErrorKind::Unauthenticated`] when it is missing, malformed, uses an
/// algorithm other than the ones above, does not cover `signed`, or does
/// not verify with any certificate.
pub fn verify_enveloped(
    source: &str,
    signed: Node<'_, '_>,
    certificates: &[Certificate],
) -> Result<(), Error> {
    let id = signed
        .attribute("ID")
        .ok_or_else(|| refused("the signed element has no ID"))?;
    let signature = only_child(signed, DSIG_NS, "Signature")
        .ok_or_else(|| refused("the element is not signed"))?;
    let info = only_child(signature, DSIG_NS, "SignedInfo")
        .ok_or_else(|| refused("the signature has no SignedInfo"))?;
    let method = |name: &str| {
        only_child(info, DSIG_NS, name).and_then(|m| m.attribute("Algorithm"))
    };
    let c14n = only_child(info, DSIG_NS, "CanonicalizationMethod").ok_or_else(
        || refused("the signature has no canonicalization method"),
    )?;
    if c14n.attribute("Algorithm") != Some(EXC_C14N) {
        return Err(refused(
            "the signature must use exclusive canonicalization",
        ));
    }
    let algorithm = method("SignatureMethod").unwrap_or_default();
    if algorithm != RSA_SHA256 && algorithm != ECDSA_SHA256 {
        return Err(refused(
            "the signature must use RSA-SHA256 or ECDSA-SHA256",
        ));
    }
    let reference =
        only_child(info, DSIG_NS, "Reference").ok_or_else(|| {
            refused("the signature must have exactly one reference")
        })?;
    if reference.attribute("URI") != Some(format!("#{id}").as_str()) {
        return Err(refused("the signature does not cover the signed element"));
    }
    let mut exclusive = None;
    let mut enveloped = false;
    if let Some(transforms) = only_child(reference, DSIG_NS, "Transforms") {
        for transform in children(transforms, DSIG_NS, "Transform") {
            match transform.attribute("Algorithm") {
                Some(ENVELOPED) => enveloped = true,
                Some(EXC_C14N) => {
                    exclusive = Some(inclusive_prefixes(transform))
                }
                _ => {
                    return Err(refused(
                        "the signature uses an unsupported transform",
                    ));
                }
            }
        }
    }
    let (Some(prefixes), true) = (exclusive, enveloped) else {
        return Err(refused(
            "the signature must use the enveloped and exclusive \
                canonicalization transforms",
        ));
    };
    let digest_method = only_child(reference, DSIG_NS, "DigestMethod")
        .and_then(|m| m.attribute("Algorithm"));
    if digest_method != Some(SHA256) {
        return Err(refused("the signature must use SHA-256 digests"));
    }
    let expected = only_child(reference, DSIG_NS, "DigestValue")
        .map(text_of)
        .transpose()?
        .and_then(|text| decode(&text))
        .ok_or_else(|| refused("the signature has no digest"))?;
    let canonical = exc_c14n(source, signed, Some(signature.id()), &prefixes)?;
    let digest = Sha256::digest(canonical.as_bytes());
    if !bool::from(digest.as_slice().ct_eq(&expected)) {
        return Err(refused("the signed element was changed after signing"));
    }
    let value = only_child(signature, DSIG_NS, "SignatureValue")
        .map(text_of)
        .transpose()?
        .and_then(|text| decode(&text))
        .ok_or_else(|| refused("the signature has no value"))?;
    let info_canonical =
        exc_c14n(source, info, None, &inclusive_prefixes(c14n))?;
    if certificates
        .iter()
        .any(|c| c.verify(algorithm, info_canonical.as_bytes(), &value))
    {
        Ok(())
    } else {
        Err(refused(
            "the signature does not match the provider's certificate",
        ))
    }
}

fn decode(text: &str) -> Option<Vec<u8>> {
    let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    STANDARD.decode(clean).ok()
}

/// The `PrefixList` of an `InclusiveNamespaces` child.
fn inclusive_prefixes(method: Node<'_, '_>) -> Vec<String> {
    only_child(method, EXC_C14N, "InclusiveNamespaces")
        .and_then(|n| n.attribute("PrefixList"))
        .map(|list| list.split_whitespace().map(str::to_owned).collect())
        .unwrap_or_default()
}

/// The text of an element that holds only text (comments and elements
/// inside it are refused, so `a@x<!---->.evil` cannot read as `a@x`).
///
/// # Errors
///
/// [`ErrorKind::Unauthenticated`] for anything but text inside.
pub fn text_of(node: Node<'_, '_>) -> Result<String, Error> {
    let mut text = String::new();
    for child in node.children() {
        match child.node_type() {
            NodeType::Text => text.push_str(child.text().unwrap_or_default()),
            _ => return Err(refused("a value holds more than text")),
        }
    }
    Ok(text)
}

/// The qualified name of an element as written.
fn element_qname<'i>(
    source: &'i str,
    node: Node<'_, 'i>,
) -> Result<&'i str, Error> {
    let start = node.range().start.saturating_add(1);
    let rest = source
        .get(start..)
        .ok_or_else(|| refused("the document cannot be read"))?;
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .ok_or_else(|| refused("the document cannot be read"))?;
    rest.get(..end)
        .ok_or_else(|| refused("the document cannot be read"))
}

fn prefix_of(qname: &str) -> &str {
    qname.split_once(':').map_or("", |(prefix, _)| prefix)
}

fn namespace_of<'a>(node: Node<'a, '_>, prefix: &str) -> Option<&'a str> {
    node.namespaces()
        .find(|ns| ns.name().unwrap_or("") == prefix)
        .map(roxmltree::Namespace::uri)
}

fn escape_text(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(c),
        }
    }
}

fn escape_attr(text: &str, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(c),
        }
    }
}

/// Exclusive XML canonicalization, without comments, of `apex`.
///
/// It covers what `apex` holds, leaving out the element `skip` (an enveloped
/// signature). `inclusive` lists prefixes rendered wherever they are in scope
/// (`#default` for the default namespace).
///
/// # Errors
///
/// [`ErrorKind::Unauthenticated`] for nesting deeper than 64 elements.
pub fn exc_c14n(
    source: &str,
    apex: Node<'_, '_>,
    skip: Option<NodeId>,
    inclusive: &[String],
) -> Result<String, Error> {
    let inclusive: Vec<&str> = inclusive
        .iter()
        .map(|p| if p == "#default" { "" } else { p.as_str() })
        .collect();
    let mut out = String::new();
    let context = Context {
        source,
        skip,
        inclusive: &inclusive,
    };
    context.element(apex, &BTreeMap::new(), 0, &mut out)?;
    Ok(out)
}

struct Context<'s, 'p> {
    source: &'s str,
    skip: Option<NodeId>,
    inclusive: &'p [&'p str],
}

impl Context<'_, '_> {
    fn element(
        &self,
        node: Node<'_, '_>,
        rendered: &BTreeMap<String, String>,
        depth: usize,
        out: &mut String,
    ) -> Result<(), Error> {
        if depth > MAX_DEPTH {
            return Err(refused("the document is nested too deeply"));
        }
        let qname = element_qname(self.source, node)?;
        let mut attributes = Vec::new();
        let mut used: BTreeSet<&str> = BTreeSet::new();
        let _ = used.insert(prefix_of(qname));
        for attr in node.attributes() {
            let attr_qname = self
                .source
                .get(attr.range_qname())
                .ok_or_else(|| refused("the document cannot be read"))?;
            let prefix = prefix_of(attr_qname);
            if !prefix.is_empty() && prefix != "xml" {
                let _ = used.insert(prefix);
            }
            attributes.push((
                attr.namespace().unwrap_or(""),
                attr.name(),
                attr_qname,
                attr.value(),
            ));
        }
        for prefix in self.inclusive {
            if namespace_of(node, prefix).is_some() {
                let _ = used.insert(prefix);
            }
        }
        let mut now = rendered.clone();
        out.push('<');
        out.push_str(qname);
        for prefix in used {
            let uri = namespace_of(node, prefix).unwrap_or("");
            let before = rendered.get(prefix).map_or("", String::as_str);
            let emit = if prefix.is_empty() {
                uri != before
            } else {
                rendered.get(prefix).map(String::as_str) != Some(uri)
            };
            if emit {
                if prefix.is_empty() {
                    out.push_str(" xmlns=\"");
                } else {
                    let _ = write!(out, " xmlns:{prefix}=\"");
                }
                escape_attr(uri, out);
                out.push('"');
                let _ = now.insert(prefix.to_owned(), uri.to_owned());
            }
        }
        attributes.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        for (_, _, attr_qname, value) in attributes {
            out.push(' ');
            out.push_str(attr_qname);
            out.push_str("=\"");
            escape_attr(value, out);
            out.push('"');
        }
        out.push('>');
        for child in node.children() {
            match child.node_type() {
                NodeType::Element if Some(child.id()) != self.skip => {
                    self.element(child, &now, depth.saturating_add(1), out)?;
                }
                NodeType::Text => {
                    escape_text(child.text().unwrap_or_default(), out)
                }
                NodeType::PI => {
                    if let Some(pi) = child.pi() {
                        out.push_str("<?");
                        out.push_str(pi.target);
                        if let Some(value) = pi.value {
                            out.push(' ');
                            out.push_str(value);
                        }
                        out.push_str("?>");
                    }
                }
                _ => {}
            }
        }
        out.push_str("</");
        out.push_str(qname);
        out.push('>');
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests fail loudly")]

    use super::*;

    fn c14n(xml: &str, path: &[&str]) -> String {
        let doc = roxmltree::Document::parse(xml).unwrap();
        let mut node = doc.root_element();
        for name in path {
            node = node
                .children()
                .find(|c| c.tag_name().name() == *name)
                .unwrap();
        }
        exc_c14n(xml, node, None, &[]).unwrap()
    }

    // Expected values come from lxml's exclusive canonicalization
    // (`etree.tostring(node, method="c14n", exclusive=True)`).
    #[test]
    fn renders_only_the_namespaces_a_subtree_uses() {
        let xml = r#"<?xml version="1.0"?>
<r:Root xmlns:r="urn:r" xmlns:a="urn:a" xmlns:unused="urn:u" xmlns="urn:d">
  <a:Child b="2" a:z="&amp;&lt;&quot;" a="1"><Inner>x &gt; y<!-- gone --></Inner><a:Empty/></a:Child>
</r:Root>"#;
        assert_eq!(
            c14n(xml, &["Child"]),
            "<a:Child xmlns:a=\"urn:a\" a=\"1\" b=\"2\" \
                a:z=\"&amp;&lt;&quot;\"><Inner xmlns=\"urn:d\">x &gt; \
                y</Inner><a:Empty></a:Empty></a:Child>"
        );
    }

    #[test]
    fn a_default_namespace_is_undeclared_only_when_rendered() {
        let xml =
            r#"<Root xmlns="urn:d"><Child xmlns=""><Leaf/></Child></Root>"#;
        assert_eq!(c14n(xml, &["Child"]), "<Child><Leaf></Leaf></Child>");
        assert_eq!(
            c14n(xml, &[]),
            "<Root xmlns=\"urn:d\"><Child \
                xmlns=\"\"><Leaf></Leaf></Child></Root>"
        );
    }

    #[test]
    fn whitespace_and_line_ends_are_kept_as_parsed() {
        let xml = "<a t=\"x&#9;y&#10;z\">\r\n  1\r\n</a>";
        assert_eq!(c14n(xml, &[]), "<a t=\"x&#x9;y&#xA;z\">\n  1\n</a>");
    }

    #[test]
    fn values_with_comments_or_elements_are_refused() {
        let xml =
            "<a><b>user@northstar.dev<!---->.evil.example</b><c>ok</c></a>";
        let doc = roxmltree::Document::parse(xml).unwrap();
        let b = doc.root_element().first_child().unwrap();
        assert!(text_of(b).is_err());
        let c = b.next_sibling().unwrap();
        assert_eq!(text_of(c).unwrap(), "ok");
    }
}
