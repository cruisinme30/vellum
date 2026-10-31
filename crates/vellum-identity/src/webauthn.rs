// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Passkeys (`WebAuthn` Level 2) as a second factor (ADR 0069, D290).
//!
//! Vellum is the relying party; its ID is the public URL's host and the
//! only origin it accepts is the public URL's origin. Registration takes
//! no attestation (`none`): a passkey proves possession, not its maker.
//! Keys are ES256 (P-256) or RS256. Every ceremony checks the challenge,
//! the type, the origin, the relying party ID hash and user presence; an
//! assertion also checks the signature and that the signature counter
//! never goes backwards (a cloned key).

use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_ASN1, RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents,
    UnparsedPublicKey,
};
use base64::Engine as _;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use ciborium::Value;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use vellum_kernel::{Error, ErrorKind};

/// COSE algorithm: ECDSA with P-256 and SHA-256.
pub const ES256: i64 = -7;
/// COSE algorithm: RSASSA-PKCS1-v1_5 with SHA-256.
pub const RS256: i64 = -257;

const FLAG_USER_PRESENT: u8 = 0x01;
const FLAG_ATTESTED: u8 = 0x40;

/// The largest client data, authenticator data or attestation accepted.
const MAX_FIELD: usize = 16 * 1024;

/// Who Vellum is to the browser.
#[derive(Debug, Clone)]
pub struct RelyingParty {
    /// The relying party ID: the public host (`vellum.northstar.dev`).
    pub id: String,
    /// The only origin ceremonies may come from
    /// (`https://vellum.northstar.dev`).
    pub origin: String,
}

/// A registered public key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "alg", rename_all = "snake_case")]
pub enum PublicKey {
    /// A P-256 point, uncompressed (`04 || x || y`), base64url.
    Es256 {
        /// The point.
        point: String,
    },
    /// An RSA key's modulus and exponent, base64url.
    Rs256 {
        /// The modulus.
        n: String,
        /// The public exponent.
        e: String,
    },
}

/// A credential as the browser returns it from
/// `navigator.credentials.create()`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Registration {
    /// The credential ID, base64url.
    pub id: String,
    /// The attestation response.
    pub response: RegistrationResponse,
}

/// The attestation response fields, base64url.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationResponse {
    /// The client data JSON.
    pub client_data_json: String,
    /// The CBOR attestation object.
    pub attestation_object: String,
}

/// A credential as the browser returns it from `navigator.credentials.get()`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Assertion {
    /// The credential ID, base64url.
    pub id: String,
    /// The assertion response.
    pub response: AssertionResponse,
}

/// The assertion response fields, base64url.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AssertionResponse {
    /// The client data JSON.
    pub client_data_json: String,
    /// The authenticator data.
    pub authenticator_data: String,
    /// The signature over the authenticator data and the client data hash.
    pub signature: String,
}

/// A verified registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registered {
    /// The credential ID, base64url without padding.
    pub id: String,
    /// The public key.
    pub key: PublicKey,
    /// The signature counter at registration.
    pub sign_count: u32,
}

fn refused(why: &str) -> Error {
    Error::new(
        ErrorKind::Unauthenticated,
        format!("the passkey was not accepted: {why}"),
    )
}

/// Decodes base64url with or without padding, within the size limit.
fn b64(text: &str) -> Result<Vec<u8>, Error> {
    if text.len() > MAX_FIELD * 4 / 3 + 4 {
        return Err(refused("a field is too large"));
    }
    URL_SAFE_NO_PAD
        .decode(text.trim_end_matches('='))
        .or_else(|_| URL_SAFE.decode(text))
        .map_err(|_| refused("a field is not base64url"))
}

/// Encodes bytes as unpadded base64url.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

#[derive(Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
    #[serde(default, rename = "crossOrigin")]
    cross_origin: bool,
}

/// Checks the client data and returns its SHA-256.
fn client_data(
    rp: &RelyingParty,
    raw: &str,
    kind: &str,
    challenge: &str,
) -> Result<[u8; 32], Error> {
    let bytes = b64(raw)?;
    let data: ClientData = serde_json::from_slice(&bytes)
        .map_err(|_| refused("the client data is not JSON"))?;
    if data.kind != kind {
        return Err(refused("the ceremony type is wrong"));
    }
    let presented = data.challenge.trim_end_matches('=');
    if !bool::from(presented.as_bytes().ct_eq(challenge.as_bytes())) {
        return Err(refused("the challenge does not match"));
    }
    if data.origin != rp.origin || data.cross_origin {
        return Err(refused("it came from another site"));
    }
    Ok(Sha256::digest(&bytes).into())
}

/// The parts of authenticator data checked for every ceremony.
struct AuthData<'a> {
    flags: u8,
    sign_count: u32,
    rest: &'a [u8],
}

fn auth_data<'a>(
    rp: &RelyingParty,
    data: &'a [u8],
) -> Result<AuthData<'a>, Error> {
    let short = || refused("the authenticator data is too short");
    let (rp_hash, rest) = data.split_at_checked(32).ok_or_else(short)?;
    let (&flags, rest) = rest.split_first().ok_or_else(short)?;
    let (count, rest) = rest.split_first_chunk::<4>().ok_or_else(short)?;
    let expected: [u8; 32] = Sha256::digest(rp.id.as_bytes()).into();
    if !bool::from(rp_hash.ct_eq(&expected)) {
        return Err(refused("it is for another site"));
    }
    if flags & FLAG_USER_PRESENT == 0 {
        return Err(refused("nobody touched the authenticator"));
    }
    Ok(AuthData {
        flags,
        sign_count: u32::from_be_bytes(*count),
        rest,
    })
}

fn map_get<'a>(map: &'a [(Value, Value)], key: &Value) -> Option<&'a Value> {
    map.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn int(value: &Value) -> Option<i128> {
    value.as_integer().map(i128::from)
}

fn bytes(value: Option<&Value>) -> Result<&[u8], Error> {
    value
        .and_then(Value::as_bytes)
        .map(Vec::as_slice)
        .ok_or_else(|| refused("the key is incomplete"))
}

/// Reads a COSE public key (RFC 9053) of an accepted algorithm.
fn cose_key(value: &Value) -> Result<PublicKey, Error> {
    let map = value
        .as_map()
        .ok_or_else(|| refused("the key is not a COSE key"))?;
    let get = |label: i64| map_get(map, &Value::Integer(label.into()));
    let kty = get(1).and_then(int);
    let alg = get(3).and_then(int);
    match (kty, alg) {
        (Some(2), Some(a)) if a == i128::from(ES256) => {
            if get(-1).and_then(int) != Some(1) {
                return Err(refused("only P-256 keys are accepted"));
            }
            let x = bytes(get(-2))?;
            let y = bytes(get(-3))?;
            if x.len() != 32 || y.len() != 32 {
                return Err(refused("the key is malformed"));
            }
            let mut point = vec![4_u8];
            point.extend_from_slice(x);
            point.extend_from_slice(y);
            Ok(PublicKey::Es256 {
                point: encode(&point),
            })
        }
        (Some(3), Some(a)) if a == i128::from(RS256) => {
            let n = bytes(get(-1))?;
            let e = bytes(get(-2))?;
            if n.len() < 256 || n.len() > 1024 || e.is_empty() || e.len() > 8 {
                return Err(refused("the RSA key size is not accepted"));
            }
            Ok(PublicKey::Rs256 {
                n: encode(n),
                e: encode(e),
            })
        }
        _ => Err(refused("only ES256 and RS256 keys are accepted")),
    }
}

/// Verifies a registration against the challenge Vellum issued.
///
/// # Errors
///
/// [`ErrorKind::Unauthenticated`] for anything that does not check out.
pub fn verify_registration(
    rp: &RelyingParty,
    challenge: &str,
    credential: &Registration,
) -> Result<Registered, Error> {
    let _ = client_data(
        rp,
        &credential.response.client_data_json,
        "webauthn.create",
        challenge,
    )?;
    let object = b64(&credential.response.attestation_object)?;
    if object.len() > MAX_FIELD {
        return Err(refused("the attestation is too large"));
    }
    let value: Value = ciborium::from_reader(object.as_slice())
        .map_err(|_| refused("the attestation is not CBOR"))?;
    let map = value
        .as_map()
        .ok_or_else(|| refused("the attestation is malformed"))?;
    let data = bytes(map_get(map, &Value::Text("authData".into())))?;
    let parsed = auth_data(rp, data)?;
    if parsed.flags & FLAG_ATTESTED == 0 {
        return Err(refused("no credential was created"));
    }
    let malformed = || refused("the credential data is malformed");
    // AAGUID (16 bytes), credential ID length (2), credential ID, COSE key.
    let (_aaguid, rest) =
        parsed.rest.split_at_checked(16).ok_or_else(malformed)?;
    let (len, rest) = rest.split_first_chunk::<2>().ok_or_else(malformed)?;
    let id_len = usize::from(u16::from_be_bytes(*len));
    if id_len == 0 || id_len > 1023 {
        return Err(malformed());
    }
    let (id, key) = rest.split_at_checked(id_len).ok_or_else(malformed)?;
    let key: Value = ciborium::from_reader(key)
        .map_err(|_| refused("the key is not CBOR"))?;
    let presented = b64(&credential.id)?;
    if presented != id {
        return Err(refused("the credential ID does not match"));
    }
    Ok(Registered {
        id: encode(id),
        key: cose_key(&key)?,
        sign_count: parsed.sign_count,
    })
}

/// Verifies an assertion by a registered key and returns the new
/// signature counter.
///
/// # Errors
///
/// [`ErrorKind::Unauthenticated`] for anything that does not check out,
/// including a counter that did not move forward.
pub fn verify_assertion(
    rp: &RelyingParty,
    challenge: &str,
    key: &PublicKey,
    stored_count: u32,
    assertion: &Assertion,
) -> Result<u32, Error> {
    let hash = client_data(
        rp,
        &assertion.response.client_data_json,
        "webauthn.get",
        challenge,
    )?;
    let data = b64(&assertion.response.authenticator_data)?;
    let parsed = auth_data(rp, &data)?;
    let signature = b64(&assertion.response.signature)?;
    let mut signed = data.clone();
    signed.extend_from_slice(&hash);
    let ok = match key {
        PublicKey::Es256 { point } => {
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, b64(point)?)
                .verify(&signed, &signature)
                .is_ok()
        }
        PublicKey::Rs256 { n, e } => RsaPublicKeyComponents {
            n: b64(n)?,
            e: b64(e)?,
        }
        .verify(&RSA_PKCS1_2048_8192_SHA256, &signed, &signature)
        .is_ok(),
    };
    if !ok {
        return Err(refused("the signature is wrong"));
    }
    // Authenticators that keep no counter always send 0.
    if (parsed.sign_count != 0 || stored_count != 0)
        && parsed.sign_count <= stored_count
    {
        return Err(refused(
            "its counter went backwards, which suggests a copied key",
        ));
    }
    Ok(parsed.sign_count)
}

#[cfg(test)]
pub(crate) mod tests {
    //! A software authenticator, for this crate's tests.

    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::signature::{
        ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair as _,
    };

    use super::*;

    /// A P-256 authenticator for one relying party.
    pub struct SoftKey {
        pair: EcdsaKeyPair,
        pub id: Vec<u8>,
        pub count: u32,
    }

    impl SoftKey {
        pub fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 = EcdsaKeyPair::generate_pkcs8(
                &ECDSA_P256_SHA256_ASN1_SIGNING,
                &rng,
            )
            .unwrap();
            let pair = EcdsaKeyPair::from_pkcs8(
                &ECDSA_P256_SHA256_ASN1_SIGNING,
                pkcs8.as_ref(),
            )
            .unwrap();
            Self {
                pair,
                id: vec![9; 16],
                count: 0,
            }
        }

        fn client_data(kind: &str, challenge: &str, origin: &str) -> Vec<u8> {
            serde_json::to_vec(&serde_json::json!({
                "type": kind, "challenge": challenge, "origin": origin,
            }))
            .unwrap()
        }

        pub fn register(
            &self,
            rp: &RelyingParty,
            challenge: &str,
        ) -> Registration {
            let point = self.pair.public_key().as_ref();
            let cose = Value::Map(vec![
                (Value::Integer(1.into()), Value::Integer(2.into())),
                (Value::Integer(3.into()), Value::Integer((-7).into())),
                (Value::Integer((-1).into()), Value::Integer(1.into())),
                (
                    Value::Integer((-2).into()),
                    Value::Bytes(point[1..33].to_vec()),
                ),
                (
                    Value::Integer((-3).into()),
                    Value::Bytes(point[33..65].to_vec()),
                ),
            ]);
            let mut data: Vec<u8> = Sha256::digest(rp.id.as_bytes()).to_vec();
            data.push(FLAG_USER_PRESENT | FLAG_ATTESTED);
            data.extend_from_slice(&self.count.to_be_bytes());
            data.extend_from_slice(&[0; 16]);
            data.extend_from_slice(
                &u16::try_from(self.id.len()).unwrap().to_be_bytes(),
            );
            data.extend_from_slice(&self.id);
            ciborium::into_writer(&cose, &mut data).unwrap();
            let object = Value::Map(vec![
                (Value::Text("fmt".into()), Value::Text("none".into())),
                (Value::Text("attStmt".into()), Value::Map(vec![])),
                (Value::Text("authData".into()), Value::Bytes(data)),
            ]);
            let mut attestation = Vec::new();
            ciborium::into_writer(&object, &mut attestation).unwrap();
            Registration {
                id: encode(&self.id),
                response: RegistrationResponse {
                    client_data_json: encode(&Self::client_data(
                        "webauthn.create",
                        challenge,
                        &rp.origin,
                    )),
                    attestation_object: encode(&attestation),
                },
            }
        }

        pub fn assert(
            &mut self,
            rp_id: &str,
            origin: &str,
            challenge: &str,
        ) -> Assertion {
            self.count += 1;
            let mut data: Vec<u8> = Sha256::digest(rp_id.as_bytes()).to_vec();
            data.push(FLAG_USER_PRESENT);
            data.extend_from_slice(&self.count.to_be_bytes());
            let client = Self::client_data("webauthn.get", challenge, origin);
            let mut signed = data.clone();
            signed.extend_from_slice(&Sha256::digest(&client));
            let signature =
                self.pair.sign(&SystemRandom::new(), &signed).unwrap();
            Assertion {
                id: encode(&self.id),
                response: AssertionResponse {
                    client_data_json: encode(&client),
                    authenticator_data: encode(&data),
                    signature: encode(signature.as_ref()),
                },
            }
        }
    }

    fn rp() -> RelyingParty {
        RelyingParty {
            id: "vellum.northstar.dev".into(),
            origin: "https://vellum.northstar.dev".into(),
        }
    }

    #[test]
    fn a_soft_key_registers_and_asserts() {
        let rp = rp();
        let mut key = SoftKey::new();
        let registered =
            verify_registration(&rp, "chal1", &key.register(&rp, "chal1"))
                .unwrap();
        assert_eq!(registered.id, encode(&key.id));
        let assertion = key.assert(&rp.id, &rp.origin, "chal2");
        let count =
            verify_assertion(&rp, "chal2", &registered.key, 0, &assertion)
                .unwrap();
        assert_eq!(count, 1);
        // The same assertion again: the counter did not move.
        assert!(
            verify_assertion(&rp, "chal2", &registered.key, count, &assertion)
                .is_err()
        );
    }

    #[test]
    fn ceremonies_for_another_site_or_challenge_are_refused() {
        let rp = rp();
        let mut key = SoftKey::new();
        assert!(
            verify_registration(&rp, "other", &key.register(&rp, "chal1"))
                .is_err()
        );
        let evil = RelyingParty {
            id: "evil.example".into(),
            origin: "https://evil.example".into(),
        };
        assert!(
            verify_registration(&rp, "chal1", &key.register(&evil, "chal1"))
                .is_err()
        );
        let registered =
            verify_registration(&rp, "c", &key.register(&rp, "c")).unwrap();
        let wrong_origin = key.assert(&rp.id, "https://evil.example", "x");
        assert!(
            verify_assertion(&rp, "x", &registered.key, 0, &wrong_origin)
                .is_err()
        );
        let wrong_rp = key.assert("evil.example", &rp.origin, "y");
        assert!(
            verify_assertion(&rp, "y", &registered.key, 0, &wrong_rp).is_err()
        );
        let other = SoftKey::new();
        let other_key =
            verify_registration(&rp, "z", &other.register(&rp, "z")).unwrap();
        let good = key.assert(&rp.id, &rp.origin, "w");
        assert!(verify_assertion(&rp, "w", &other_key.key, 0, &good).is_err());
        let mut tampered = key.assert(&rp.id, &rp.origin, "v");
        tampered.response.client_data_json = encode(
            br#"{"type":"webauthn.get","challenge":"v","origin":"https://vellum.northstar.dev","x":1}"#
        );
        assert!(
            verify_assertion(&rp, "v", &registered.key, 0, &tampered).is_err()
        );
    }
}
