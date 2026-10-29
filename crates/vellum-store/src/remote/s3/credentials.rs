// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Where the S3 adapter gets its signing keys (ADR 0044, D186).
//!
//! Static keys still work, but on Kubernetes and AWS the pod's own
//! identity is preferred, so no long-lived key has to be stored anywhere.
//! [`CredentialSource::from_vars`] picks the first source that is set up,
//! in the order the AWS SDKs use:
//!
//! 1. `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` (and the optional
//!    `AWS_SESSION_TOKEN`): static keys.
//! 2. `AWS_ROLE_ARN` with `AWS_WEB_IDENTITY_TOKEN_FILE`: STS
//!    `AssumeRoleWithWebIdentity`. EKS sets these for IRSA; `MinIO`'s STS
//!    accepts the same call on any Kubernetes when `AWS_ENDPOINT_URL_STS`
//!    points at it.
//! 3. `AWS_CONTAINER_CREDENTIALS_FULL_URI` or `_RELATIVE_URI`, with
//!    `AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE` or `_TOKEN`: the container
//!    credential endpoint of EKS Pod Identity and ECS tasks.
//! 4. The EC2 instance profile through `IMDSv2`, but only when the bucket
//!    is on AWS itself (no `VELLUM_S3_ENDPOINT`) and
//!    `AWS_EC2_METADATA_DISABLED` is not `true`.
//!
//! Temporary credentials are cached and fetched again five minutes before
//! they expire. One refresh runs at a time; if it fails while the old keys
//! are still valid, they are used and the failure is logged.

use std::{
    fmt,
    path::PathBuf,
    time::{Duration, SystemTime},
};

use reqwest::{Client, Method, StatusCode};
use serde::Deserialize;
use tokio::sync::Mutex;

use super::{S3Credentials, list};
use crate::{
    error::{StoreError, StoreResult},
    remote::Secret,
};

/// Refresh this long before temporary credentials expire.
const REFRESH_MARGIN: Duration = Duration::from_mins(5);

/// The `IMDSv2` session token lifetime asked for, in seconds.
const IMDS_TOKEN_TTL: &str = "21600";

/// The ECS task credential host, which `_RELATIVE_URI` is relative to.
const ECS_HOST: &str = "http://169.254.170.2";

/// The default instance metadata endpoint.
const IMDS_ENDPOINT: &str = "http://169.254.169.254";

/// Hosts a plain-`http` container credential URL may name: the ECS and EKS
/// Pod Identity agents. Anything else must be `https` or loopback.
const CONTAINER_HOSTS: [&str; 3] = ["169.254.170.2", "169.254.170.23", "[fd00:ec2::23]"];

/// Where an [`S3Store`](super::S3Store) gets the keys it signs with.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum CredentialSource {
    /// Fixed keys, such as from a Kubernetes Secret.
    Static(S3Credentials),
    /// STS `AssumeRoleWithWebIdentity` with a projected service account
    /// token (EKS IRSA, `MinIO` STS).
    WebIdentity(WebIdentity),
    /// The container credential endpoint (EKS Pod Identity, ECS).
    Container(Container),
    /// The EC2 instance profile, through `IMDSv2`.
    InstanceProfile(InstanceProfile),
}

/// Settings for [`CredentialSource::WebIdentity`].
#[derive(Clone, Debug)]
pub struct WebIdentity {
    /// The role to assume, such as `arn:aws:iam::111122223333:role/vellum`.
    pub role_arn: String,
    /// The file holding the identity token. It is read again on every
    /// refresh, because the kubelet rotates it.
    pub token_file: PathBuf,
    /// The session name recorded in `CloudTrail`.
    pub session_name: String,
    /// The STS endpoint, such as `https://sts.eu-west-1.amazonaws.com`.
    pub sts_endpoint: String,
}

/// Settings for [`CredentialSource::Container`].
#[derive(Clone, Debug)]
pub struct Container {
    /// The full credential URL.
    pub url: String,
    /// The authorization token, when given inline.
    pub token: Option<Secret>,
    /// The file holding the authorization token, read on every refresh.
    pub token_file: Option<PathBuf>,
}

/// Settings for [`CredentialSource::InstanceProfile`].
#[derive(Clone, Debug)]
pub struct InstanceProfile {
    /// The metadata endpoint, normally `http://169.254.169.254`.
    pub endpoint: String,
}

impl From<S3Credentials> for CredentialSource {
    fn from(credentials: S3Credentials) -> Self {
        Self::Static(credentials)
    }
}

impl CredentialSource {
    /// Picks the first configured source; see the module documentation for
    /// the order. `region` names the regional STS endpoint, and `on_aws` is
    /// false for custom endpoints (`MinIO`, R2), which never fall back to
    /// the instance profile.
    ///
    /// # Errors
    /// [`StoreError::Backend`] when a key id is set without its secret, a
    /// container URL is not allowed, or no source is configured. Secret
    /// values are never quoted.
    pub fn from_vars(
        var: impl Fn(&str) -> Option<String>,
        region: &str,
        on_aws: bool,
    ) -> StoreResult<Self> {
        let get = |name: &str| var(name).filter(|v| !v.trim().is_empty());
        let fail = |why: String| StoreError::Backend(format!("s3: {why}"));
        if let Some(access_key_id) = get("AWS_ACCESS_KEY_ID") {
            let secret = get("AWS_SECRET_ACCESS_KEY").ok_or_else(|| {
                fail("AWS_ACCESS_KEY_ID is set but AWS_SECRET_ACCESS_KEY is not".into())
            })?;
            return Ok(Self::Static(S3Credentials {
                access_key_id,
                secret_access_key: Secret::new(secret),
                session_token: get("AWS_SESSION_TOKEN").map(Secret::new),
            }));
        }
        if let (Some(role_arn), Some(token_file)) =
            (get("AWS_ROLE_ARN"), get("AWS_WEB_IDENTITY_TOKEN_FILE"))
        {
            let region = get("AWS_REGION")
                .or_else(|| get("AWS_DEFAULT_REGION"))
                .unwrap_or_else(|| region.to_owned());
            let sts_endpoint = get("AWS_ENDPOINT_URL_STS")
                .or_else(|| get("AWS_ENDPOINT_URL"))
                .map_or_else(
                    || format!("https://sts.{region}.amazonaws.com"),
                    |e| e.trim_end_matches('/').to_owned(),
                );
            return Ok(Self::WebIdentity(WebIdentity {
                role_arn,
                token_file: token_file.into(),
                session_name: get("AWS_ROLE_SESSION_NAME").unwrap_or_else(session_name),
                sts_endpoint,
            }));
        }
        let full = get("AWS_CONTAINER_CREDENTIALS_FULL_URI");
        let relative = get("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI");
        if full.is_some() || relative.is_some() {
            let url = match (full, relative) {
                (_, Some(rel)) => format!("{ECS_HOST}/{}", rel.trim_start_matches('/')),
                (Some(full), None) => {
                    check_container_url(&full).map_err(fail)?;
                    full
                }
                (None, None) => unreachable!("one of them is set"),
            };
            return Ok(Self::Container(Container {
                url,
                token: get("AWS_CONTAINER_AUTHORIZATION_TOKEN").map(Secret::new),
                token_file: get("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE").map(PathBuf::from),
            }));
        }
        let imds_off =
            get("AWS_EC2_METADATA_DISABLED").is_some_and(|v| v.eq_ignore_ascii_case("true"));
        if on_aws && !imds_off {
            return Ok(Self::InstanceProfile(InstanceProfile {
                endpoint: get("AWS_EC2_METADATA_SERVICE_ENDPOINT").map_or_else(
                    || IMDS_ENDPOINT.to_owned(),
                    |e| e.trim_end_matches('/').to_owned(),
                ),
            }));
        }
        Err(fail(
            "no credentials: set AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY, or use workload \
             identity (AWS_ROLE_ARN with AWS_WEB_IDENTITY_TOKEN_FILE, or \
             AWS_CONTAINER_CREDENTIALS_FULL_URI)"
                .into(),
        ))
    }

    /// A short name for logs and metrics: `static`, `web_identity`,
    /// `container` or `instance_profile`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Static(_) => "static",
            Self::WebIdentity(_) => "web_identity",
            Self::Container(_) => "container",
            Self::InstanceProfile(_) => "instance_profile",
        }
    }
}

/// `vellum-<milliseconds since the epoch>`, unique enough per process.
fn session_name() -> String {
    let millis = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    format!("vellum-{millis}")
}

/// A full container URL must be `https`, loopback, or one of the AWS
/// agents' link-local addresses, as the AWS SDKs require, so a planted
/// variable cannot send the token to an arbitrary host.
fn check_container_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| format!("AWS_CONTAINER_CREDENTIALS_FULL_URI is not a URL: {e}"))?;
    let host = parsed.host_str().unwrap_or_default();
    let allowed = match parsed.scheme() {
        "https" => true,
        "http" => {
            CONTAINER_HOSTS.contains(&host)
                || host == "localhost"
                || host
                    .trim_start_matches('[')
                    .trim_end_matches(']')
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        }
        _ => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "AWS_CONTAINER_CREDENTIALS_FULL_URI must be https, loopback or an AWS agent \
             address, not {host:?}"
        ))
    }
}

/// Keys with the time they stop working (`None` for static keys).
#[derive(Clone)]
struct Cached {
    credentials: S3Credentials,
    expires: Option<SystemTime>,
}

impl Cached {
    fn fresh(&self, now: SystemTime) -> bool {
        self.expires.is_none_or(|t| now + REFRESH_MARGIN < t)
    }

    fn valid(&self, now: SystemTime) -> bool {
        self.expires.is_none_or(|t| now < t)
    }
}

/// Hands out current keys for a [`CredentialSource`], fetching temporary
/// ones when they are missing or about to expire.
pub(super) struct Provider {
    source: CredentialSource,
    client: Client,
    cache: Mutex<Option<Cached>>,
}

impl fmt::Debug for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Provider")
            .field("source", &self.source.kind())
            .finish_non_exhaustive()
    }
}

impl Provider {
    pub(super) fn new(source: CredentialSource) -> StoreResult<Self> {
        let cache = match &source {
            CredentialSource::Static(credentials) => Some(Cached {
                credentials: credentials.clone(),
                expires: None,
            }),
            _ => None,
        };
        // Credential endpoints are close by (link-local agents) or
        // regional; a slow one should fail the request, not hang it.
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("vellum/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| StoreError::Backend(format!("s3: cannot build client: {e}")))?;
        Ok(Self {
            source,
            client,
            cache: Mutex::new(cache),
        })
    }

    /// The keys to sign the next request with.
    pub(super) async fn get(&self) -> StoreResult<S3Credentials> {
        let mut cache = self.cache.lock().await;
        let now = SystemTime::now();
        if let Some(cached) = cache.as_ref().filter(|c| c.fresh(now)) {
            return Ok(cached.credentials.clone());
        }
        match self.fetch().await {
            Ok(fetched) => {
                tracing::debug!(source = self.source.kind(), "fetched s3 credentials");
                let credentials = fetched.credentials.clone();
                *cache = Some(fetched);
                Ok(credentials)
            }
            Err(error) => match cache.as_ref().filter(|c| c.valid(now)) {
                Some(old) => {
                    tracing::warn!(%error, source = self.source.kind(),
                        "cannot refresh s3 credentials; using the current ones until they expire");
                    Ok(old.credentials.clone())
                }
                None => Err(error),
            },
        }
    }

    #[tracing::instrument(name = "s3.credentials", skip(self), fields(source = self.source.kind()), err)]
    async fn fetch(&self) -> StoreResult<Cached> {
        match &self.source {
            CredentialSource::Static(credentials) => Ok(Cached {
                credentials: credentials.clone(),
                expires: None,
            }),
            CredentialSource::WebIdentity(web) => self.web_identity(web).await,
            CredentialSource::Container(container) => self.container(container).await,
            CredentialSource::InstanceProfile(imds) => self.instance_profile(imds).await,
        }
    }

    async fn web_identity(&self, web: &WebIdentity) -> StoreResult<Cached> {
        let token = read_token(&web.token_file, "AWS_WEB_IDENTITY_TOKEN_FILE").await?;
        let form = [
            ("Action", "AssumeRoleWithWebIdentity"),
            ("Version", "2011-06-15"),
            ("RoleArn", web.role_arn.as_str()),
            ("RoleSessionName", web.session_name.as_str()),
            ("WebIdentityToken", token.expose()),
        ];
        let response = self
            .client
            .post(format!("{}/", web.sts_endpoint))
            .form(&form)
            .send()
            .await
            .map_err(|e| unreachable_source("STS", &e))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| unreachable_source("STS", &e))?;
        if !status.is_success() {
            let code = list::element_text(&body, "Code").unwrap_or_default();
            let message = list::element_text(&body, "Message").unwrap_or_default();
            return Err(StoreError::Backend(format!(
                "s3: STS refused AssumeRoleWithWebIdentity for {}: {status} {code} {message}",
                web.role_arn
            )));
        }
        let field = |tag: &str| {
            list::element_text(&body, tag)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| StoreError::Backend(format!("s3: STS answer has no {tag}")))
        };
        Ok(Cached {
            credentials: S3Credentials {
                access_key_id: field("AccessKeyId")?,
                secret_access_key: Secret::new(field("SecretAccessKey")?),
                session_token: Some(Secret::new(field("SessionToken")?)),
            },
            expires: list::element_text(&body, "Expiration")
                .as_deref()
                .and_then(list::parse_timestamp),
        })
    }

    async fn container(&self, container: &Container) -> StoreResult<Cached> {
        let token = match (&container.token_file, &container.token) {
            (Some(file), _) => {
                Some(read_token(file, "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE").await?)
            }
            (None, token) => token.clone(),
        };
        let mut request = self.client.get(&container.url);
        if let Some(token) = &token {
            request = request.header(reqwest::header::AUTHORIZATION, token.expose());
        }
        let response = request
            .send()
            .await
            .map_err(|e| unreachable_source("the container credential endpoint", &e))?;
        json_credentials(response, "the container credential endpoint").await
    }

    async fn instance_profile(&self, imds: &InstanceProfile) -> StoreResult<Cached> {
        let what = "the instance metadata service";
        let token = self
            .client
            .request(Method::PUT, format!("{}/latest/api/token", imds.endpoint))
            .header("x-aws-ec2-metadata-token-ttl-seconds", IMDS_TOKEN_TTL)
            .send()
            .await
            .map_err(|e| unreachable_source(what, &e))?;
        if !token.status().is_success() {
            return Err(StoreError::Backend(format!(
                "s3: {what} refused a session token: {}",
                token.status()
            )));
        }
        let token = token
            .text()
            .await
            .map_err(|e| unreachable_source(what, &e))?;
        let base = format!(
            "{}/latest/meta-data/iam/security-credentials/",
            imds.endpoint
        );
        let roles = self
            .client
            .get(&base)
            .header("x-aws-ec2-metadata-token", &token)
            .send()
            .await
            .map_err(|e| unreachable_source(what, &e))?;
        if roles.status() == StatusCode::NOT_FOUND {
            return Err(StoreError::Backend(format!(
                "s3: this instance has no instance profile; attach a role or set {}",
                "AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY"
            )));
        }
        let roles = roles
            .text()
            .await
            .map_err(|e| unreachable_source(what, &e))?;
        let role = roles
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .ok_or_else(|| {
                StoreError::Backend(format!("s3: {what} lists no instance profile role"))
            })?;
        let response = self
            .client
            .get(format!("{base}{role}"))
            .header("x-aws-ec2-metadata-token", &token)
            .send()
            .await
            .map_err(|e| unreachable_source(what, &e))?;
        json_credentials(response, what).await
    }
}

/// The JSON shape the container endpoint and `IMDS` both answer with.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct JsonCredentials {
    access_key_id: String,
    secret_access_key: String,
    token: Option<String>,
    expiration: Option<String>,
}

async fn json_credentials(response: reqwest::Response, what: &str) -> StoreResult<Cached> {
    let status = response.status();
    if !status.is_success() {
        return Err(StoreError::Backend(format!("s3: {what} answered {status}")));
    }
    let body: JsonCredentials = response.json().await.map_err(|e| {
        StoreError::Backend(format!(
            "s3: {what} sent malformed credentials: {}",
            e.without_url()
        ))
    })?;
    Ok(Cached {
        credentials: S3Credentials {
            access_key_id: body.access_key_id,
            secret_access_key: Secret::new(body.secret_access_key),
            session_token: body.token.filter(|t| !t.is_empty()).map(Secret::new),
        },
        expires: body.expiration.as_deref().and_then(list::parse_timestamp),
    })
}

async fn read_token(file: &std::path::Path, var: &str) -> StoreResult<Secret> {
    let text = tokio::fs::read_to_string(file).await.map_err(|e| {
        StoreError::Backend(format!("s3: cannot read {var} {}: {e}", file.display()))
    })?;
    Ok(Secret::new(text.trim().to_owned()))
}

fn unreachable_source(what: &str, e: &reqwest::Error) -> StoreError {
    StoreError::Backend(format!("s3: cannot reach {what}: {e}"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn static_keys_win() {
        let source = CredentialSource::from_vars(
            vars(&[
                ("AWS_ACCESS_KEY_ID", "id"),
                ("AWS_SECRET_ACCESS_KEY", "secret"),
                ("AWS_ROLE_ARN", "arn:aws:iam::1:role/x"),
                ("AWS_WEB_IDENTITY_TOKEN_FILE", "/t"),
            ]),
            "us-east-1",
            true,
        )
        .unwrap();
        assert_eq!(source.kind(), "static");
    }

    #[test]
    fn a_key_id_without_its_secret_is_refused() {
        let error =
            CredentialSource::from_vars(vars(&[("AWS_ACCESS_KEY_ID", "id")]), "us-east-1", true)
                .unwrap_err();
        assert!(error.to_string().contains("AWS_SECRET_ACCESS_KEY"));
    }

    #[test]
    fn web_identity_uses_the_regional_sts_endpoint() {
        let source = CredentialSource::from_vars(
            vars(&[
                ("AWS_ROLE_ARN", "arn:aws:iam::1:role/x"),
                ("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/token"),
                ("AWS_REGION", "eu-west-1"),
            ]),
            "us-east-1",
            true,
        )
        .unwrap();
        let CredentialSource::WebIdentity(web) = source else {
            panic!("expected web identity");
        };
        assert_eq!(web.sts_endpoint, "https://sts.eu-west-1.amazonaws.com");
        assert!(web.session_name.starts_with("vellum-"));
    }

    #[test]
    fn container_urls_are_checked() {
        let ok = |url: &str| {
            CredentialSource::from_vars(
                vars(&[("AWS_CONTAINER_CREDENTIALS_FULL_URI", url)]),
                "us-east-1",
                true,
            )
        };
        assert!(ok("http://169.254.170.23/v1/credentials").is_ok());
        assert!(ok("http://127.0.0.1:8080/creds").is_ok());
        assert!(ok("https://creds.example.com/x").is_ok());
        assert!(ok("http://evil.example.com/x").is_err());
        assert!(ok("http://10.0.0.5/x").is_err());
        let relative = CredentialSource::from_vars(
            vars(&[(
                "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
                "/v2/credentials/abc",
            )]),
            "us-east-1",
            true,
        )
        .unwrap();
        let CredentialSource::Container(c) = relative else {
            panic!("expected container");
        };
        assert_eq!(c.url, "http://169.254.170.2/v2/credentials/abc");
    }

    #[test]
    fn instance_profile_only_on_aws() {
        let on_aws = CredentialSource::from_vars(vars(&[]), "us-east-1", true).unwrap();
        assert_eq!(on_aws.kind(), "instance_profile");
        let custom = CredentialSource::from_vars(vars(&[]), "us-east-1", false).unwrap_err();
        assert!(custom.to_string().contains("no credentials"));
        let disabled = CredentialSource::from_vars(
            vars(&[("AWS_EC2_METADATA_DISABLED", "true")]),
            "us-east-1",
            true,
        );
        assert!(disabled.is_err());
    }

    #[test]
    fn cache_freshness() {
        let now = SystemTime::now();
        let cached = |expires| Cached {
            credentials: S3Credentials {
                access_key_id: "a".into(),
                secret_access_key: Secret::new("b"),
                session_token: None,
            },
            expires,
        };
        assert!(cached(None).fresh(now));
        assert!(cached(Some(now + Duration::from_hours(1))).fresh(now));
        let soon = cached(Some(now + Duration::from_mins(2)));
        assert!(!soon.fresh(now));
        assert!(soon.valid(now));
        assert!(!cached(Some(now - Duration::from_secs(1))).valid(now));
    }
}
