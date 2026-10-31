# SPDX-FileCopyrightText: 2026 The Vellum contributors
# SPDX-License-Identifier: AGPL-3.0-only

"""Builds SAML responses signed with lxml's exclusive c14n, independent of Vellum's.

Run: python3 make_fixtures.py . (needs lxml and cryptography). Keys are thrown away.
"""
import base64, datetime, hashlib, sys
from lxml import etree
from cryptography import x509
from cryptography.x509.oid import NameOID
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa, ec, padding
from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature

out = sys.argv[1]
DS = "http://www.w3.org/2000/09/xmldsig#"

def cert_for(key):
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "idp.northstar.dev")])
    now = datetime.datetime(2026, 1, 1)
    return (x509.CertificateBuilder().subject_name(name).issuer_name(name)
            .public_key(key.public_key()).serial_number(1)
            .not_valid_before(now).not_valid_after(now + datetime.timedelta(days=3650))
            .sign(key, hashes.SHA256()))

RESPONSE = """<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol"
    xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion"
    xmlns:xs="http://www.w3.org/2001/XMLSchema"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    ID="_resp-{tag}" Version="2.0" IssueInstant="2026-10-30T12:00:00Z"
    Destination="https://vellum.northstar.dev/saml/acs" InResponseTo="_python-request">
  <saml:Issuer>https://idp.northstar.dev</saml:Issuer>
  <samlp:Status><samlp:StatusCode Value="urn:oasis:names:tc:SAML:2.0:status:Success"/></samlp:Status>
  <saml:Assertion ID="_assert-{tag}" Version="2.0" IssueInstant="2026-10-30T12:00:00Z">
    <saml:Issuer>https://idp.northstar.dev</saml:Issuer>
    <saml:Subject>
      <saml:NameID Format="urn:oasis:names:tc:SAML:1.1:nameid-format:unspecified">00u1priya</saml:NameID>
      <saml:SubjectConfirmation Method="urn:oasis:names:tc:SAML:2.0:cm:bearer">
        <saml:SubjectConfirmationData InResponseTo="_python-request" NotOnOrAfter="2026-10-30T12:05:00Z"
            Recipient="https://vellum.northstar.dev/saml/acs"/>
      </saml:SubjectConfirmation>
    </saml:Subject>
    <saml:Conditions NotBefore="2026-10-30T11:59:00Z" NotOnOrAfter="2026-10-30T12:05:00Z">
      <saml:AudienceRestriction><saml:Audience>https://vellum.northstar.dev/saml/metadata</saml:Audience></saml:AudienceRestriction>
    </saml:Conditions>
    <saml:AuthnStatement AuthnInstant="2026-10-30T12:00:00Z" SessionIndex="_s1">
      <saml:AuthnContext><saml:AuthnContextClassRef>urn:oasis:names:tc:SAML:2.0:ac:classes:PasswordProtectedTransport</saml:AuthnContextClassRef></saml:AuthnContext>
    </saml:AuthnStatement>
    <saml:AttributeStatement>
      <saml:Attribute Name="email" NameFormat="urn:oasis:names:tc:SAML:2.0:attrname-format:basic">
        <saml:AttributeValue xsi:type="xs:string">Priya@Northstar.dev</saml:AttributeValue>
      </saml:Attribute>
      <saml:Attribute Name="displayName"><saml:AttributeValue xsi:type="xs:string">Priya Raman &amp; co</saml:AttributeValue></saml:Attribute>
    </saml:AttributeStatement>
  </saml:Assertion>
</samlp:Response>"""

def sign(tag, key, alg):
    root = etree.fromstring(RESPONSE.format(tag=tag).encode())
    assertion = root.find("{urn:oasis:names:tc:SAML:2.0:assertion}Assertion")
    canon = etree.tostring(assertion, method="c14n", exclusive=True, with_comments=False,
                           inclusive_ns_prefixes=["xs"])
    digest = base64.b64encode(hashlib.sha256(canon).digest()).decode()
    sig = etree.fromstring(f"""<ds:Signature xmlns:ds="{DS}"><ds:SignedInfo><ds:CanonicalizationMethod Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"/><ds:SignatureMethod Algorithm="{alg}"/><ds:Reference URI="#_assert-{tag}"><ds:Transforms><ds:Transform Algorithm="http://www.w3.org/2000/09/xmldsig#enveloped-signature"/><ds:Transform Algorithm="http://www.w3.org/2001/10/xml-exc-c14n#"><ec:InclusiveNamespaces xmlns:ec="http://www.w3.org/2001/10/xml-exc-c14n#" PrefixList="xs"/></ds:Transform></ds:Transforms><ds:DigestMethod Algorithm="http://www.w3.org/2001/04/xmlenc#sha256"/><ds:DigestValue>{digest}</ds:DigestValue></ds:Reference></ds:SignedInfo><ds:SignatureValue/></ds:Signature>""")
    assertion.insert(1, sig)
    info = sig.find(f"{{{DS}}}SignedInfo")
    info_canon = etree.tostring(info, method="c14n", exclusive=True, with_comments=False)
    if isinstance(key, rsa.RSAPrivateKey):
        value = key.sign(info_canon, padding.PKCS1v15(), hashes.SHA256())
    else:
        r, s = decode_dss_signature(key.sign(info_canon, ec.ECDSA(hashes.SHA256())))
        value = r.to_bytes(32, "big") + s.to_bytes(32, "big")
    sig.find(f"{{{DS}}}SignatureValue").text = "\n" + base64.encodebytes(value).decode()
    return etree.tostring(root, xml_declaration=True, encoding="UTF-8")

rsa_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
ec_key = ec.generate_private_key(ec.SECP256R1())
for tag, key, alg in [("rsa", rsa_key, "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256"),
                      ("ecdsa", ec_key, "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256")]:
    open(f"{out}/{tag}-response.xml", "wb").write(sign(tag, key, alg))
    open(f"{out}/{tag}-cert.pem", "wb").write(cert_for(key).public_bytes(serialization.Encoding.PEM))
