// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The S3 adapter signs with workload identity (ADR 0044, D186): keys from
//! STS web identity, the container credential endpoint and `IMDSv2`, all
//! served here by one fake, together with a fake bucket that only accepts
//! requests signed with those temporary keys.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, head, post, put},
};
use vellum_store::{
    BlobPath, BlobStore,
    remote::s3::{Addressing, CredentialSource, S3Config, S3Store},
};

const KEY_ID: &str = "ASIATEMPORARY";
const SESSION: &str = "session-token-1";

#[derive(Default)]
struct Fake {
    sts_calls: AtomicUsize,
    container_calls: AtomicUsize,
    imds_calls: AtomicUsize,
    /// Expiry handed out, as an ISO 8601 string.
    expiration: std::sync::Mutex<String>,
}

#[allow(clippy::many_single_char_names)]
fn iso(t: SystemTime) -> String {
    let secs = t.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
    let days = secs / 86_400;
    let rem = secs % 86_400;
    // Civil from days (Howard Hinnant).
    let z = i64::try_from(days).unwrap() + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

async fn sts(State(fake): State<Arc<Fake>>, body: String) -> impl IntoResponse {
    let _ = fake.sts_calls.fetch_add(1, Ordering::SeqCst);
    let form: HashMap<String, String> = body
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    if form.get("Action").map(String::as_str)
        != Some("AssumeRoleWithWebIdentity")
        || form.get("WebIdentityToken").map(String::as_str)
            != Some("projected-jwt")
    {
        return (
            StatusCode::FORBIDDEN,
            "<ErrorResponse><Error><Code>AccessDenied</Code><Message>bad \
                token</Message></Error></ErrorResponse>"
                .to_owned(),
        );
    }
    let expiration = fake.expiration.lock().unwrap().clone();
    (
        StatusCode::OK,
        format!(
            "<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResul\
             t><Credentials><AccessKeyId>{KEY_ID}</AccessKeyId><SecretAccessKey\
             >temp-secret</SecretAccessKey><SessionToken>{SESSION}</SessionToke\
             n><Expiration>{expiration}</Expiration></Credentials></AssumeRoleW\
             ithWebIdentityResult></AssumeRoleWithWebIdentityResponse>"
        ),
    )
}

fn json_credentials(fake: &Fake) -> String {
    let expiration = fake.expiration.lock().unwrap().clone();
    format!("{{\"AccessKeyId\":\"{KEY_ID}\",\"SecretAccessKey\":\"temp-secret\"\
        ,\"Token\":\"{SESSION}\",\"Expiration\":\"{expiration}\"}}")
}

async fn container(
    State(fake): State<Arc<Fake>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let _ = fake.container_calls.fetch_add(1, Ordering::SeqCst);
    if headers.get("authorization").and_then(|v| v.to_str().ok())
        != Some("pod-identity-token")
    {
        return (StatusCode::UNAUTHORIZED, String::new());
    }
    (StatusCode::OK, json_credentials(&fake))
}

async fn imds_token(headers: HeaderMap) -> impl IntoResponse {
    if headers.contains_key("x-aws-ec2-metadata-token-ttl-seconds") {
        (StatusCode::OK, "imds-session")
    } else {
        (StatusCode::BAD_REQUEST, "")
    }
}

fn imds_ok(headers: &HeaderMap) -> bool {
    headers
        .get("x-aws-ec2-metadata-token")
        .and_then(|v| v.to_str().ok())
        == Some("imds-session")
}

async fn imds_roles(headers: HeaderMap) -> impl IntoResponse {
    if imds_ok(&headers) {
        (StatusCode::OK, "vellum-node\n")
    } else {
        (StatusCode::UNAUTHORIZED, "")
    }
}

async fn imds_role(
    State(fake): State<Arc<Fake>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let _ = fake.imds_calls.fetch_add(1, Ordering::SeqCst);
    if !imds_ok(&headers) {
        return (StatusCode::UNAUTHORIZED, String::new());
    }
    (StatusCode::OK, json_credentials(&fake))
}

/// The bucket: only requests signed with the temporary keys get through.
fn signed(headers: &HeaderMap) -> bool {
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let token = headers
        .get("x-amz-security-token")
        .and_then(|v| v.to_str().ok());
    auth.contains(&format!("Credential={KEY_ID}/")) && token == Some(SESSION)
}

async fn bucket_head(headers: HeaderMap) -> impl IntoResponse {
    if !signed(&headers) {
        return (StatusCode::FORBIDDEN, HeaderMap::new());
    }
    let mut out = HeaderMap::new();
    let _ = out.insert("etag", "\"abc\"".parse().unwrap());
    let _ = out.insert("content-length", "5".parse().unwrap());
    (StatusCode::OK, out)
}

async fn bucket_put(headers: HeaderMap, _body: Bytes) -> impl IntoResponse {
    if !signed(&headers) {
        return (StatusCode::FORBIDDEN, HeaderMap::new());
    }
    let mut out = HeaderMap::new();
    let _ = out.insert("etag", "\"def\"".parse().unwrap());
    (StatusCode::OK, out)
}

async fn start(fake: Arc<Fake>) -> String {
    let app = Router::new()
        .route("/", post(sts))
        .route("/creds", get(container))
        .route("/latest/api/token", put(imds_token))
        .route(
            "/latest/meta-data/iam/security-credentials/",
            get(imds_roles),
        )
        .route(
            "/latest/meta-data/iam/security-credentials/vellum-node",
            get(imds_role),
        )
        .route("/notes/a.md", head(bucket_head).put(bucket_put))
        .with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    format!("http://{addr}")
}

fn store(endpoint: &str, credentials: CredentialSource) -> S3Store {
    S3Store::new(S3Config {
        endpoint: endpoint.to_owned(),
        region: "us-east-1".into(),
        bucket: "notes".into(),
        prefix: None,
        addressing: Addressing::Path,
        credentials,
    })
    .unwrap()
}

fn vars(pairs: Vec<(&'static str, String)>) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<&str, String> = pairs.into_iter().collect();
    move |k| map.get(k).cloned()
}

fn fake_expiring_in(d: Duration) -> Arc<Fake> {
    let fake = Arc::new(Fake::default());
    *fake.expiration.lock().unwrap() = iso(SystemTime::now() + d);
    fake
}

#[tokio::test]
async fn web_identity_signs_with_sts_keys_and_caches_them() {
    let fake = fake_expiring_in(Duration::from_hours(1));
    let endpoint = start(Arc::clone(&fake)).await;
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("token");
    std::fs::write(&token, "projected-jwt\n").unwrap();
    let source = CredentialSource::from_vars(
        vars(vec![
            (
                "AWS_ROLE_ARN",
                "arn:aws:iam::111122223333:role/vellum".into(),
            ),
            ("AWS_WEB_IDENTITY_TOKEN_FILE", token.display().to_string()),
            ("AWS_ENDPOINT_URL_STS", endpoint.clone()),
        ]),
        "us-east-1",
        false,
    )
    .unwrap();
    assert_eq!(source.kind(), "web_identity");
    let s3 = store(&endpoint, source);
    let path = BlobPath::new("a.md").unwrap();
    let meta = s3.head(&path).await.unwrap();
    assert_eq!(meta.size, 5);
    let _ = s3
        .put(
            &path,
            Bytes::from_static(b"hello"),
            vellum_store::Precondition::None,
        )
        .await
        .unwrap();
    // Valid for an hour: one STS call serves both requests.
    assert_eq!(fake.sts_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn credentials_close_to_expiry_are_fetched_again() {
    // Inside the five-minute refresh margin from the start.
    let fake = fake_expiring_in(Duration::from_mins(2));
    let endpoint = start(Arc::clone(&fake)).await;
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("token");
    std::fs::write(&token, "projected-jwt").unwrap();
    let source = CredentialSource::from_vars(
        vars(vec![
            ("AWS_ROLE_ARN", "arn:aws:iam::1:role/vellum".into()),
            ("AWS_WEB_IDENTITY_TOKEN_FILE", token.display().to_string()),
            ("AWS_ENDPOINT_URL_STS", endpoint.clone()),
        ]),
        "us-east-1",
        false,
    )
    .unwrap();
    let s3 = store(&endpoint, source);
    let path = BlobPath::new("a.md").unwrap();
    let _ = s3.head(&path).await.unwrap();
    let _ = s3.head(&path).await.unwrap();
    assert_eq!(fake.sts_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_refused_web_identity_names_the_sts_error() {
    let fake = fake_expiring_in(Duration::from_hours(1));
    let endpoint = start(Arc::clone(&fake)).await;
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("token");
    std::fs::write(&token, "someone-elses-jwt").unwrap();
    let source = CredentialSource::from_vars(
        vars(vec![
            ("AWS_ROLE_ARN", "arn:aws:iam::1:role/vellum".into()),
            ("AWS_WEB_IDENTITY_TOKEN_FILE", token.display().to_string()),
            ("AWS_ENDPOINT_URL_STS", endpoint.clone()),
        ]),
        "us-east-1",
        false,
    )
    .unwrap();
    let error = store(&endpoint, source)
        .head(&BlobPath::new("a.md").unwrap())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("AccessDenied"), "{error}");
    assert!(!error.contains("someone-elses-jwt"), "{error}");
}

#[tokio::test]
async fn pod_identity_reads_the_token_file() {
    let fake = fake_expiring_in(Duration::from_hours(1));
    let endpoint = start(Arc::clone(&fake)).await;
    let dir = tempfile::tempdir().unwrap();
    let token = dir.path().join("eks-pod-identity-token");
    std::fs::write(&token, "pod-identity-token").unwrap();
    let source = CredentialSource::from_vars(
        vars(vec![
            (
                "AWS_CONTAINER_CREDENTIALS_FULL_URI",
                format!("{endpoint}/creds"),
            ),
            (
                "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE",
                token.display().to_string(),
            ),
        ]),
        "us-east-1",
        false,
    )
    .unwrap();
    assert_eq!(source.kind(), "container");
    let s3 = store(&endpoint, source);
    assert_eq!(
        s3.head(&BlobPath::new("a.md").unwrap()).await.unwrap().size,
        5
    );
    assert_eq!(fake.container_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn instance_profile_uses_imdsv2() {
    let fake = fake_expiring_in(Duration::from_hours(6));
    let endpoint = start(Arc::clone(&fake)).await;
    let source = CredentialSource::from_vars(
        vars(vec![(
            "AWS_EC2_METADATA_SERVICE_ENDPOINT",
            endpoint.clone(),
        )]),
        "us-east-1",
        true,
    )
    .unwrap();
    assert_eq!(source.kind(), "instance_profile");
    let s3 = store(&endpoint, source);
    assert_eq!(
        s3.head(&BlobPath::new("a.md").unwrap()).await.unwrap().size,
        5
    );
    assert_eq!(fake.imds_calls.load(Ordering::SeqCst), 1);
}
