// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Each provider against a small fake of its key service: envelopes seal,
//! open and re-wrap; associated data, credentials and errors behave.

use std::sync::{Arc, Mutex};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use vellum_crypto::{CryptoError, KeyProvider, kek_of, open, rewrap, seal};
use vellum_kms::{
    AwsCredentialProvider, AwsCredentials, AwsKms, GcpKms, MetadataServerToken,
    StaticToken, VaultTransit,
};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// One request as the fake service saw it.
#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

type Handler = dyn Fn(&Seen) -> (u16, Value) + Send + Sync;

/// A keep-alive HTTP/1.1 server on localhost that answers with `handler`
/// and records every request.
async fn serve(
    handler: Arc<Handler>,
) -> Result<(String, Arc<Mutex<Vec<Seen>>>), BoxError> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let log = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&log);
    drop(tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let handler = Arc::clone(&handler);
            let seen = Arc::clone(&seen);
            drop(tokio::spawn(async move {
                let _ = connection(stream, handler.as_ref(), &seen).await;
            }));
        }
    }));
    Ok((base, log))
}

async fn connection(
    stream: tokio::net::TcpStream,
    handler: &Handler,
    log: &Mutex<Vec<Seen>>,
) -> Result<(), BoxError> {
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let path = parts.next().unwrap_or_default().to_owned();
        let mut headers = Vec::new();
        let mut length = 0_usize;
        loop {
            let mut header = String::new();
            let _ = reader.read_line(&mut header).await?;
            let header = header.trim_end();
            if header.is_empty() {
                break;
            }
            if let Some((name, value)) = header.split_once(':') {
                let name = name.trim().to_ascii_lowercase();
                let value = value.trim().to_owned();
                if name == "content-length" {
                    length = value.parse()?;
                }
                headers.push((name, value));
            }
        }
        let mut body = vec![0_u8; length];
        let _ = reader.read_exact(&mut body).await?;
        let seen = Seen {
            method,
            path,
            headers,
            body,
        };
        let (status, reply) = handler(&seen);
        if let Ok(mut log) = log.lock() {
            log.push(seen);
        }
        // A JSON string is sent as plain text (instance metadata answers).
        let (kind, reply) = match reply {
            Value::String(text) => ("text/plain", text),
            other => ("application/json", other.to_string()),
        };
        let response = format!(
            "HTTP/1.1 {status} X\r\ncontent-type: {kind}\r\n\
                content-length: {}\r\n\r\n{reply}",
            reply.len()
        );
        reader.get_mut().write_all(response.as_bytes()).await?;
    }
}

fn b64(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

fn unb64(text: &str) -> Vec<u8> {
    STANDARD.decode(text).unwrap_or_default()
}

fn field<'a>(body: &'a Value, name: &str) -> &'a str {
    body.get(name).and_then(Value::as_str).unwrap_or_default()
}

/// A fake "ciphertext": the key, associated data and plaintext in a JSON
/// object, so decryption can check the key and associated data match.
fn fake_seal(key: &str, aad: &str, plaintext: &str) -> String {
    b64(json!({ "key": key, "aad": aad, "pt": plaintext })
        .to_string()
        .as_bytes())
}

fn fake_open(key: &str, aad: &str, ciphertext: &str) -> Option<String> {
    let inner: Value = serde_json::from_slice(&unb64(ciphertext)).ok()?;
    (field(&inner, "key") == key && field(&inner, "aad") == aad)
        .then(|| field(&inner, "pt").to_owned())
}

async fn round_trip<K: KeyProvider>(old: &K, new: &K) -> Result<(), BoxError> {
    let sealed = seal(old, b"Northstar roadmap").await?;
    assert_eq!(kek_of(&sealed)?, old.current_kek());
    assert_eq!(open(old, &sealed).await?, b"Northstar roadmap");

    let moved = rewrap(new, &sealed).await?.ok_or("rewrap did nothing")?;
    assert_eq!(kek_of(&moved)?, new.current_kek());
    assert_eq!(open(new, &moved).await?, b"Northstar roadmap");
    assert!(rewrap(new, &moved).await?.is_none());
    Ok(())
}

fn vault_handler() -> Arc<Handler> {
    Arc::new(|seen: &Seen| {
        if seen.header("x-vault-token") != Some("s.root") {
            return (403, json!({ "errors": ["permission denied"] }));
        }
        let body = seen.json();
        let aad = field(&body, "associated_data");
        if let Some(key) = seen.path.strip_prefix("/v1/transit/encrypt/") {
            let ct = format!(
                "vault:v1:{}",
                fake_seal(key, aad, field(&body, "plaintext"))
            );
            return (200, json!({ "data": { "ciphertext": ct } }));
        }
        if let Some(key) = seen.path.strip_prefix("/v1/transit/decrypt/") {
            let ct = field(&body, "ciphertext").trim_start_matches("vault:v1:");
            return fake_open(key, aad, ct).map_or_else(
                || {
                    (
                        400,
                        json!({ "errors": ["cipher: message authentication \
                            failed"] }),
                    )
                },
                |pt| (200, json!({ "data": { "plaintext": pt } })),
            );
        }
        (404, json!({ "errors": [] }))
    })
}

#[tokio::test]
async fn vault_transit_seals_opens_and_rotates() -> Result<(), BoxError> {
    let (base, log) = serve(vault_handler()).await?;
    let old = VaultTransit::new(&base, "s.root", "transit", "vellum-2026")?;
    let new = VaultTransit::new(&base, "s.root", "transit", "vellum-2027")?
        .with_namespace("team");
    assert_eq!(old.current_kek().as_str(), "vault:transit/vellum-2026");
    round_trip(&old, &new).await?;

    let log = log.lock().map_err(|_| "poisoned")?.clone();
    let first = log.first().ok_or("no requests")?;
    assert_eq!(first.method, "POST");
    assert_eq!(first.path, "/v1/transit/encrypt/vellum-2026");
    let aad =
        String::from_utf8(unb64(field(&first.json(), "associated_data")))?;
    assert_eq!(aad, "vellum/dek/v1:vault:transit/vellum-2026");
    assert!(
        log.iter()
            .any(|s| s.header("x-vault-namespace") == Some("team"))
    );
    Ok(())
}

#[tokio::test]
async fn vault_errors_are_provider_errors_without_secrets()
-> Result<(), BoxError> {
    let (base, _) = serve(vault_handler()).await?;
    let provider = VaultTransit::new(&base, "s.wrong", "transit", "vellum")?;
    let Err(CryptoError::Provider(message)) = seal(&provider, b"x").await
    else {
        return Err("expected a provider error".into());
    };
    assert!(message.contains("403"), "{message}");
    assert!(!message.contains("s.wrong"), "{message}");
    Ok(())
}

#[tokio::test]
async fn vault_refuses_a_dek_moved_to_another_kek_id() -> Result<(), BoxError> {
    let (base, _) = serve(vault_handler()).await?;
    let provider = VaultTransit::new(&base, "s.root", "transit", "vellum")?;
    let dek = vellum_crypto::SecretKey::generate()?;
    let mut wrapped = provider.wrap(&dek).await?;
    wrapped.kek_id = vellum_crypto::KekId::new("vault:transit/other")?;
    assert!(provider.unwrap(&wrapped).await.is_err());
    Ok(())
}

fn aws_handler() -> Arc<Handler> {
    aws_handler_for("AKIDEXAMPLE", "session")
}

/// The KMS fake, accepting calls signed with `access_key` and `session`.
fn aws_handler_for(
    access_key: &'static str,
    session: &'static str,
) -> Arc<Handler> {
    Arc::new(move |seen: &Seen| {
        let signed = seen.header("authorization").is_some_and(|a| {
            a.starts_with(&format!("AWS4-HMAC-SHA256 Credential={access_key}/"))
                && a.contains("/eu-west-1/kms/aws4_request")
                && a.contains(
                    "SignedHeaders=content-type;host;x-amz-date;x-amz-security-\
                        token;x-amz-target",
                )
        });
        if !signed || seen.header("x-amz-security-token") != Some(session) {
            return (400, json!({ "__type": "IncompleteSignatureException" }));
        }
        let body = seen.json();
        let key = field(&body, "KeyId");
        let context = body
            .pointer("/EncryptionContext/vellum")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match seen.header("x-amz-target") {
            Some("TrentService.Encrypt") => {
                let blob = fake_seal(key, context, field(&body, "Plaintext"));
                (
                    200,
                    json!({
                        "CiphertextBlob": b64(blob.as_bytes()),
                        "KeyId": key
                    }),
                )
            }
            Some("TrentService.Decrypt") => {
                let blob =
                    String::from_utf8(unb64(field(&body, "CiphertextBlob")))
                        .unwrap_or_default();
                fake_open(key, context, &blob).map_or_else(
                    || (400, json!({ "__type": "InvalidCiphertextException" })),
                    |pt| (200, json!({ "Plaintext": pt, "KeyId": key })),
                )
            }
            _ => (400, json!({ "__type": "UnknownOperationException" })),
        }
    })
}

#[tokio::test]
async fn aws_kms_seals_opens_and_rotates() -> Result<(), BoxError> {
    let (base, log) = serve(aws_handler()).await?;
    let credentials =
        AwsCredentials::new("AKIDEXAMPLE", "secret", Some("session".into()));
    let old =
        AwsKms::new("eu-west-1", "alias/vellum-2026", credentials.clone())?
            .with_endpoint(&base);
    let new = AwsKms::new(
        "eu-west-1",
        "arn:aws:kms:eu-west-1:111122223333:key/1234abcd-12ab-34cd-56ef-1234567\
            890ab",
        credentials,
    )?
    .with_endpoint(&base);
    assert_eq!(
        old.current_kek().as_str(),
        "aws-kms:eu-west-1:alias/vellum-2026"
    );
    round_trip(&old, &new).await?;

    let log = log.lock().map_err(|_| "poisoned")?.clone();
    let first = log.first().ok_or("no requests")?;
    assert_eq!(
        first.header("content-type"),
        Some("application/x-amz-json-1.1")
    );
    assert_eq!(
        first
            .json()
            .pointer("/EncryptionContext/vellum")
            .and_then(Value::as_str),
        Some("vellum/dek/v1:aws-kms:eu-west-1:alias/vellum-2026")
    );
    Ok(())
}

fn gcp_handler() -> Arc<Handler> {
    Arc::new(|seen: &Seen| {
        if seen.path == "/token" {
            return if seen.header("metadata-flavor") == Some("Google") {
                (
                    200,
                    json!({
                        "access_token": "ya29.token",
                        "expires_in": 3600,
                        "token_type": "Bearer"
                    }),
                )
            } else {
                (403, json!({}))
            };
        }
        if seen.header("authorization") != Some("Bearer ya29.token") {
            return (401, json!({ "error": { "status": "UNAUTHENTICATED" } }));
        }
        let body = seen.json();
        let aad = field(&body, "additionalAuthenticatedData");
        let path = seen.path.trim_start_matches("/v1/");
        if let Some(name) = path.strip_suffix(":encrypt") {
            let ct = fake_seal(name, aad, field(&body, "plaintext"));
            return (
                200,
                json!({ "name": name, "ciphertext": b64(ct.as_bytes()) }),
            );
        }
        if let Some(name) = path.strip_suffix(":decrypt") {
            let ct = String::from_utf8(unb64(field(&body, "ciphertext")))
                .unwrap_or_default();
            return fake_open(name, aad, &ct).map_or_else(
                || (400, json!({ "error": { "status": "INVALID_ARGUMENT" } })),
                |pt| (200, json!({ "plaintext": pt })),
            );
        }
        (404, json!({}))
    })
}

#[tokio::test]
async fn gcp_kms_seals_opens_and_rotates_with_cached_metadata_tokens()
-> Result<(), BoxError> {
    let (base, log) = serve(gcp_handler()).await?;
    let ring =
        "projects/northstar/locations/europe-west1/keyRings/vellum/cryptoKeys";
    let tokens = || MetadataServerToken::with_url(format!("{base}/token"));
    let old = GcpKms::new(&format!("{ring}/kek-2026"), tokens()?)?
        .with_endpoint(&base);
    let new = GcpKms::new(&format!("{ring}/kek-2027"), tokens()?)?
        .with_endpoint(&base);
    round_trip(&old, &new).await?;

    let log = log.lock().map_err(|_| "poisoned")?.clone();
    let token_fetches = log.iter().filter(|s| s.path == "/token").count();
    assert_eq!(token_fetches, 2, "one fetch per provider, then cached");
    assert!(
        log.iter()
            .any(|s| s.path == format!("/v1/{ring}/kek-2026:encrypt"))
    );
    Ok(())
}

#[tokio::test]
async fn gcp_kms_with_a_static_token() -> Result<(), BoxError> {
    let (base, _) = serve(gcp_handler()).await?;
    let name =
        "projects/northstar/locations/global/keyRings/vellum/cryptoKeys/kek";
    let good =
        GcpKms::new(name, StaticToken::new("ya29.token"))?.with_endpoint(&base);
    let sealed = seal(&good, b"hello").await?;
    assert_eq!(open(&good, &sealed).await?, b"hello");

    let bad =
        GcpKms::new(name, StaticToken::new("expired"))?.with_endpoint(&base);
    assert!(matches!(
        open(&bad, &sealed).await,
        Err(CryptoError::Provider(m)) if m.contains("401")
    ));
    Ok(())
}

#[tokio::test]
async fn providers_refuse_kek_ids_of_other_backends() -> Result<(), BoxError> {
    let (base, _) = serve(vault_handler()).await?;
    let vault = VaultTransit::new(&base, "s.root", "transit", "vellum")?;
    let aws = AwsKms::new(
        "eu-west-1",
        "alias/k",
        AwsCredentials::new("a", "b", None),
    )?
    .with_endpoint(&base);
    let sealed = seal(&vault, b"x").await?;
    assert!(matches!(
        open(&aws, &sealed).await,
        Err(CryptoError::UnknownKek(_))
    ));
    Ok(())
}

/// A KMS fake that also answers STS, container and instance metadata
/// requests, and only accepts KMS calls signed with the role's key.
fn aws_role_handler() -> Arc<Handler> {
    let kms = aws_handler_for("ASIAROLE", "role-session");
    Arc::new(move |seen: &Seen| {
        let creds = json!({
            "AccessKeyId": "ASIAROLE",
            "SecretAccessKey": "role-secret",
            "Token": "role-session",
            "Expiration": "2999-01-01T00:00:00Z",
        });
        match (seen.method.as_str(), seen.path.as_str()) {
            ("POST", "/sts/") => {
                let body = String::from_utf8_lossy(&seen.body).into_owned();
                if !body.contains("Action=AssumeRoleWithWebIdentity")
                    || !body.contains("WebIdentityToken=oidc-token-1")
                    || !body.contains(
                        "RoleArn=arn%3Aaws%3Aiam%3A%3A11112222333\
                        3%3Arole%2Fvellum",
                    )
                {
                    return (
                        400,
                        json!({ "Error": { "Code": "InvalidIdentityTok\
                        en" } }),
                    );
                }
                (
                    200,
                    json!({ "AssumeRoleWithWebIdentityResponse": {
                    "AssumeRoleWithWebIdentityResult": { "Credentials": {
                        "AccessKeyId": "ASIAROLE",
                        "SecretAccessKey": "role-secret",
                        "SessionToken": "role-session",
                        "Expiration": 32_503_680_000_u64,
                    } }
                } }),
                )
            }
            ("GET", "/creds") => {
                if seen.header("authorization") == Some("pod-identity-token") {
                    (200, creds)
                } else {
                    (401, json!({ "message": "bad token" }))
                }
            }
            ("PUT", "/latest/api/token") => {
                if seen
                    .header("x-aws-ec2-metadata-token-ttl-seconds")
                    .is_some()
                {
                    (200, Value::String("imds-session".into()))
                } else {
                    (400, Value::Null)
                }
            }
            ("GET", path)
                if path.starts_with(
                    "/latest/meta-data/iam/security-credentials/",
                ) =>
            {
                if seen.header("x-aws-ec2-metadata-token")
                    != Some("imds-session")
                {
                    return (401, Value::Null);
                }
                if path.ends_with('/') {
                    (200, Value::String("vellum-node\n".into()))
                } else if path.ends_with("/vellum-node") {
                    let mut body = creds;
                    if let Some(map) = body.as_object_mut() {
                        let _ = map.insert("Code".into(), json!("Success"));
                    }
                    (200, body)
                } else {
                    (404, Value::Null)
                }
            }
            _ => kms(seen),
        }
    })
}

async fn role_round_trip(
    credentials: AwsCredentialProvider,
    base: &str,
) -> Result<(), BoxError> {
    let kms = AwsKms::new("eu-west-1", "alias/vellum", credentials)?
        .with_endpoint(base);
    let sealed = seal(&kms, b"role credentials").await?;
    assert_eq!(open(&kms, &sealed).await?, b"role credentials");
    Ok(())
}

#[tokio::test]
async fn aws_kms_uses_web_identity_without_static_keys() -> Result<(), BoxError>
{
    let (base, log) = serve(aws_role_handler()).await?;
    let dir = std::env::temp_dir()
        .join(format!("vellum-irsa-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let token = dir.join("token");
    std::fs::write(&token, "oidc-token-1\n")?;
    let credentials = AwsCredentialProvider::from_vars(|name| match name {
        "AWS_WEB_IDENTITY_TOKEN_FILE" => Some(token.display().to_string()),
        "AWS_ROLE_ARN" => Some("arn:aws:iam::111122223333:role/vellum".into()),
        "AWS_ENDPOINT_URL_STS" => Some(format!("{base}/sts")),
        _ => None,
    })?;
    assert_eq!(credentials.kind(), "web-identity");
    role_round_trip(credentials, &base).await?;
    let log = log.lock().map_err(|_| "poisoned")?.clone();
    // Cached: one STS call for both KMS calls, and the token never in a URL.
    assert_eq!(log.iter().filter(|s| s.path == "/sts/").count(), 1);
    assert!(log.iter().all(|s| !s.path.contains("oidc-token")));
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

#[tokio::test]
async fn aws_kms_uses_container_credentials() -> Result<(), BoxError> {
    let (base, _) = serve(aws_role_handler()).await?;
    let dir = std::env::temp_dir()
        .join(format!("vellum-pod-identity-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let token = dir.join("token");
    std::fs::write(&token, "pod-identity-token")?;
    let credentials = AwsCredentialProvider::container(
        &format!("{base}/creds"),
        Some(token.clone()),
    )?;
    role_round_trip(credentials, &base).await?;

    std::fs::write(&token, "stale")?;
    let refused = AwsCredentialProvider::container(
        &format!("{base}/creds"),
        Some(token),
    )?;
    let error = AwsKms::new("eu-west-1", "alias/vellum", refused)?
        .with_endpoint(&base)
        .wrap(&vellum_crypto::SecretKey::generate()?)
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(error.contains("401"), "{error}");
    assert!(!error.contains("stale"), "{error}");
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

#[tokio::test]
async fn aws_kms_uses_imds_v2_instance_roles() -> Result<(), BoxError> {
    let (base, log) = serve(aws_role_handler()).await?;
    role_round_trip(AwsCredentialProvider::imds(&base)?, &base).await?;
    let log = log.lock().map_err(|_| "poisoned")?.clone();
    let first = log.first().ok_or("no requests")?;
    assert_eq!(
        (first.method.as_str(), first.path.as_str()),
        ("PUT", "/latest/api/token")
    );
    Ok(())
}
