// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Where AWS KMS calls get their credentials (ADR 0025, D259).
//!
//! Production deployments should not hold long-lived access keys. The
//! [`AwsCredentialProvider::from_env`] chain follows the AWS SDKs, in order:
//!
//! 1. `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY` (plus
//!    `AWS_SESSION_TOKEN`), for development and for keys a sidecar rotates.
//! 2. Web identity: `AWS_WEB_IDENTITY_TOKEN_FILE` and `AWS_ROLE_ARN`, which
//!    EKS IAM roles for service accounts (IRSA) set. The token file is read
//!    again on every refresh, because Kubernetes rotates it.
//! 3. Container credentials: `AWS_CONTAINER_CREDENTIALS_RELATIVE_URI` (ECS
//!    task roles) or `AWS_CONTAINER_CREDENTIALS_FULL_URI` with
//!    `AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE` (EKS Pod Identity).
//! 4. The EC2 instance metadata service, version 2 only (session token
//!    first), unless `AWS_EC2_METADATA_DISABLED=true`.
//!
//! Temporary credentials are cached and refreshed five minutes before they
//! expire. Secrets are zeroized and never logged or put in error text.

use std::fmt;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use vellum_crypto::CryptoError;
use zeroize::Zeroizing;

use crate::aws::AwsCredentials;
use crate::http;

const BACKEND: &str = "aws-credentials";

/// Refresh temporary credentials this long before they expire.
const REFRESH_MARGIN: Duration = Duration::from_mins(5);

/// The ECS agent's credentials endpoint, for relative URIs.
const ECS_HOST: &str = "http://169.254.170.2";

/// Plain-HTTP hosts a full container-credentials URI may name: loopback,
/// the ECS endpoint and the EKS Pod Identity agent. Anything else must be
/// HTTPS, so the authorization token never crosses the network in clear.
const CONTAINER_HOSTS: [&str; 5] = [
    "127.0.0.1",
    "localhost",
    "[::1]",
    "169.254.170.2",
    "169.254.170.23",
];

/// The EKS Pod Identity agent's `IPv6` address.
const EKS_IPV6_HOST: &str = "[fd00:ec2::23]";

/// The instance metadata service.
const IMDS_ENDPOINT: &str = "http://169.254.169.254";

/// Lifetime asked for an `IMDSv2` session token.
const IMDS_TOKEN_TTL: &str = "21600";

/// Session name for web identity, when `AWS_ROLE_SESSION_NAME` is unset.
const SESSION_NAME: &str = "vellum";

/// Where credentials come from.
enum Source {
    Fixed(AwsCredentials),
    WebIdentity {
        role_arn: String,
        token_file: PathBuf,
        session_name: String,
        sts_endpoint: String,
    },
    Container {
        url: String,
        token: ContainerToken,
    },
    Imds {
        endpoint: String,
    },
}

/// The authorization header value for container credentials.
enum ContainerToken {
    None,
    Inline(Zeroizing<String>),
    /// Read on every refresh; EKS rotates it.
    File(PathBuf),
}

/// Supplies AWS credentials, refreshing temporary ones before they expire.
pub struct AwsCredentialProvider {
    source: Source,
    client: reqwest::Client,
    cached: Mutex<Option<(AwsCredentials, Option<SystemTime>)>>,
}

impl fmt::Debug for AwsCredentialProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AwsCredentialProvider")
            .field("source", &self.kind())
            .finish_non_exhaustive()
    }
}

impl From<AwsCredentials> for AwsCredentialProvider {
    fn from(credentials: AwsCredentials) -> Self {
        Self::fixed(credentials)
    }
}

impl AwsCredentialProvider {
    fn with_source(source: Source) -> Result<Self, CryptoError> {
        Ok(Self {
            source,
            client: http::client()?,
            cached: Mutex::new(None),
        })
    }

    /// Always the same credentials.
    #[must_use]
    pub fn fixed(credentials: AwsCredentials) -> Self {
        Self {
            source: Source::Fixed(credentials),
            // A fixed source never makes a request; the default client is
            // never used, so its settings do not matter.
            client: reqwest::Client::new(),
            cached: Mutex::new(None),
        }
    }

    /// Assumes `role_arn` with the OIDC token in `token_file`, through STS
    /// at `sts_endpoint` (for example `https://sts.eu-west-1.amazonaws.com`).
    ///
    /// # Errors
    /// [`CryptoError::Provider`] if the HTTP client cannot be built.
    pub fn web_identity(
        role_arn: impl Into<String>,
        token_file: impl Into<PathBuf>,
        session_name: impl Into<String>,
        sts_endpoint: impl Into<String>,
    ) -> Result<Self, CryptoError> {
        Self::with_source(Source::WebIdentity {
            role_arn: role_arn.into(),
            token_file: token_file.into(),
            session_name: session_name.into(),
            sts_endpoint: sts_endpoint.into().trim_end_matches('/').to_owned(),
        })
    }

    /// Container credentials from `url`, sending the token read from
    /// `token_file` (if any) as the `Authorization` header.
    ///
    /// # Errors
    /// [`CryptoError::Provider`] if `url` is plain HTTP to a host other than
    /// loopback or the ECS and EKS agents, or the client cannot be built.
    pub fn container(
        url: &str,
        token_file: Option<PathBuf>,
    ) -> Result<Self, CryptoError> {
        check_container_url(url)?;
        Self::with_source(Source::Container {
            url: url.to_owned(),
            token: token_file
                .map_or(ContainerToken::None, ContainerToken::File),
        })
    }

    /// Instance-role credentials from `IMDSv2` at `endpoint`
    /// (`http://169.254.169.254` on EC2).
    ///
    /// # Errors
    /// [`CryptoError::Provider`] if the HTTP client cannot be built.
    pub fn imds(endpoint: impl Into<String>) -> Result<Self, CryptoError> {
        Self::with_source(Source::Imds {
            endpoint: endpoint.into().trim_end_matches('/').to_owned(),
        })
    }

    /// The first source the environment configures, in the SDK order
    /// described in the module documentation.
    ///
    /// # Errors
    /// [`CryptoError::Provider`] for a half-configured source (a token file
    /// without a role, an unsafe container URL), or when the HTTP client
    /// cannot be built.
    pub fn from_env() -> Result<Self, CryptoError> {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    /// [`Self::from_env`] with the variables passed in.
    ///
    /// # Errors
    /// As [`Self::from_env`].
    pub fn from_vars(
        var: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, CryptoError> {
        let var = |name: &str| var(name).filter(|v| !v.trim().is_empty());
        let missing = |name: &str, because: &str| {
            CryptoError::Provider(format!(
                "{BACKEND}: {name} is not set but {because} is"
            ))
        };
        if let Some(id) = var("AWS_ACCESS_KEY_ID") {
            let secret = var("AWS_SECRET_ACCESS_KEY").ok_or_else(|| {
                missing("AWS_SECRET_ACCESS_KEY", "AWS_ACCESS_KEY_ID")
            })?;
            return Ok(Self::fixed(AwsCredentials::new(
                id,
                secret,
                var("AWS_SESSION_TOKEN"),
            )));
        }
        if let Some(token_file) = var("AWS_WEB_IDENTITY_TOKEN_FILE") {
            let role = var("AWS_ROLE_ARN").ok_or_else(|| {
                missing("AWS_ROLE_ARN", "AWS_WEB_IDENTITY_TOKEN_FILE")
            })?;
            let session = var("AWS_ROLE_SESSION_NAME")
                .unwrap_or_else(|| SESSION_NAME.into());
            return Self::web_identity(
                role,
                token_file,
                session,
                sts_endpoint(&var),
            );
        }
        if let Some(relative) = var("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI") {
            if !relative.starts_with('/') {
                return Err(CryptoError::Provider(format!(
                    "{BACKEND}: AWS_CONTAINER_CREDENTIALS_RELATIVE_URI must \
                        start with /"
                )));
            }
            return Self::with_source(Source::Container {
                url: format!("{ECS_HOST}{relative}"),
                token: ContainerToken::None,
            });
        }
        if let Some(full) = var("AWS_CONTAINER_CREDENTIALS_FULL_URI") {
            check_container_url(&full)?;
            let token = match (
                var("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE"),
                var("AWS_CONTAINER_AUTHORIZATION_TOKEN"),
            ) {
                (Some(file), _) => ContainerToken::File(file.into()),
                (None, Some(token)) => {
                    ContainerToken::Inline(Zeroizing::new(token))
                }
                (None, None) => ContainerToken::None,
            };
            return Self::with_source(Source::Container { url: full, token });
        }
        if var("AWS_EC2_METADATA_DISABLED")
            .is_some_and(|v| v.eq_ignore_ascii_case("true"))
        {
            return Err(CryptoError::Provider(format!(
                "{BACKEND}: no AWS credentials are configured and the instance \
                 metadata service is disabled"
            )));
        }
        let endpoint = var("AWS_EC2_METADATA_SERVICE_ENDPOINT")
            .unwrap_or_else(|| IMDS_ENDPOINT.into());
        Self::imds(endpoint)
    }

    /// Which kind of source this is, for logs: `static`, `web-identity`,
    /// `container` or `imds`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self.source {
            Source::Fixed(_) => "static",
            Source::WebIdentity { .. } => "web-identity",
            Source::Container { .. } => "container",
            Source::Imds { .. } => "imds",
        }
    }

    /// Credentials valid for at least the next few minutes.
    ///
    /// # Errors
    /// [`CryptoError::Provider`] if the source cannot be reached, refuses,
    /// or answers without credentials.
    pub async fn credentials(&self) -> Result<AwsCredentials, CryptoError> {
        if let Source::Fixed(credentials) = &self.source {
            return Ok(credentials.clone());
        }
        if let Some(fresh) = self.cached() {
            return Ok(fresh);
        }
        let started = std::time::Instant::now();
        let fetched = match &self.source {
            Source::Fixed(credentials) => Ok((credentials.clone(), None)),
            Source::WebIdentity {
                role_arn,
                token_file,
                session_name,
                sts_endpoint,
            } => {
                self.web_identity_now(
                    role_arn,
                    token_file,
                    session_name,
                    sts_endpoint,
                )
                .await
            }
            Source::Container { url, token } => {
                self.container_now(url, token).await
            }
            Source::Imds { endpoint } => self.imds_now(endpoint).await,
        };
        let (credentials, expires) =
            http::record(BACKEND, self.kind(), started, fetched)?;
        if let Ok(mut guard) = self.cached.lock() {
            *guard = Some((credentials.clone(), expires));
        }
        Ok(credentials)
    }

    fn cached(&self) -> Option<AwsCredentials> {
        let (credentials, expires) = self.cached.lock().ok()?.clone()?;
        let fresh =
            expires.is_none_or(|at| SystemTime::now() + REFRESH_MARGIN < at);
        fresh.then_some(credentials)
    }

    async fn web_identity_now(
        &self,
        role_arn: &str,
        token_file: &PathBuf,
        session_name: &str,
        sts_endpoint: &str,
    ) -> Result<(AwsCredentials, Option<SystemTime>), CryptoError> {
        let token = read_secret(token_file, "AWS_WEB_IDENTITY_TOKEN_FILE")?;
        // A form body, not a query string, so the token stays out of
        // access logs. AssumeRoleWithWebIdentity is not signed.
        let form = Zeroizing::new(format!(
            "Action=AssumeRoleWithWebIdentity&Version=2011-06-15&RoleArn={}\
             &RoleSessionName={}&WebIdentityToken={}",
            form_encode(role_arn),
            form_encode(session_name),
            form_encode(&token),
        ));
        let request = self
            .client
            .post(format!("{sts_endpoint}/"))
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/json")
            .body(form.as_bytes().to_vec());
        let body = http::send_json(BACKEND, request).await?;
        let result = body
            .pointer(
                "/AssumeRoleWithWebIdentityResponse/AssumeRoleWithWebIdentityRe\
                    sult/Credentials",
            )
            .ok_or_else(|| {
                CryptoError::Provider(format!(
                    "{BACKEND}: STS answered without \
                    credentials"
                ))
            })?;
        parse_credentials(result, "SessionToken")
    }

    async fn container_now(
        &self,
        url: &str,
        token: &ContainerToken,
    ) -> Result<(AwsCredentials, Option<SystemTime>), CryptoError> {
        let mut request = self.client.get(url);
        match token {
            ContainerToken::None => {}
            ContainerToken::Inline(value) => {
                request = request.header("authorization", value.as_str());
            }
            ContainerToken::File(path) => {
                let value = read_secret(
                    path,
                    "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE",
                )?;
                request = request.header("authorization", value.as_str());
            }
        }
        let body = http::send_json(BACKEND, request).await?;
        parse_credentials(&body, "Token")
    }

    async fn imds_now(
        &self,
        endpoint: &str,
    ) -> Result<(AwsCredentials, Option<SystemTime>), CryptoError> {
        // IMDSv2 only: a session token first, then every read carries it.
        let token = Zeroizing::new(
            http::send_text(
                BACKEND,
                self.client
                    .put(format!("{endpoint}/latest/api/token"))
                    .header(
                        "x-aws-ec2-metadata-token-ttl-seconds",
                        IMDS_TOKEN_TTL,
                    ),
            )
            .await?,
        );
        let base =
            format!("{endpoint}/latest/meta-data/iam/security-credentials/");
        let roles = http::send_text(
            BACKEND,
            self.client
                .get(&base)
                .header("x-aws-ec2-metadata-token", token.as_str()),
        )
        .await?;
        let role = roles
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .filter(|role| http::safe_segment(role, &['+', '=', ',', '@']))
            .ok_or_else(|| {
                CryptoError::Provider(format!(
                    "{BACKEND}: this instance has no IAM role"
                ))
            })?;
        let body = http::send_json(
            BACKEND,
            self.client
                .get(format!("{base}{role}"))
                .header("x-aws-ec2-metadata-token", token.as_str()),
        )
        .await?;
        if let Some(code) = body.get("Code").and_then(Value::as_str)
            && code != "Success"
        {
            return Err(CryptoError::Provider(format!(
                "{BACKEND}: instance metadata answered {code}"
            )));
        }
        parse_credentials(&body, "Token")
    }
}

/// The STS endpoint: `AWS_ENDPOINT_URL_STS`, else the regional endpoint
/// for `AWS_REGION` or `AWS_DEFAULT_REGION`, else the global one.
fn sts_endpoint(var: &impl Fn(&str) -> Option<String>) -> String {
    if let Some(url) = var("AWS_ENDPOINT_URL_STS") {
        return url;
    }
    var("AWS_REGION")
        .or_else(|| var("AWS_DEFAULT_REGION"))
        .filter(|region| http::safe_segment(region, &[]))
        .map_or_else(
            || "https://sts.amazonaws.com".to_owned(),
            |region| format!("https://sts.{region}.amazonaws.com"),
        )
}

/// Plain HTTP only to loopback or the ECS and EKS agents.
fn check_container_url(url: &str) -> Result<(), CryptoError> {
    if url.starts_with("https://") {
        return Ok(());
    }
    let host = url
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .map(|authority| {
            if authority.starts_with('[') {
                authority
                    .split_once(']')
                    .map_or(authority, |(h, _)| &authority[..=h.len()])
            } else {
                authority.split(':').next().unwrap_or(authority)
            }
        })
        .unwrap_or_default();
    if CONTAINER_HOSTS.contains(&host) || host == EKS_IPV6_HOST {
        Ok(())
    } else {
        Err(CryptoError::Provider(format!(
            "{BACKEND}: AWS_CONTAINER_CREDENTIALS_FULL_URI must be HTTPS, or \
             HTTP to loopback or the ECS or EKS credentials agent"
        )))
    }
}

/// Reads a token file, trimming surrounding whitespace. The error names
/// the variable, never the file's content.
fn read_secret(
    path: &PathBuf,
    variable: &str,
) -> Result<Zeroizing<String>, CryptoError> {
    let raw = Zeroizing::new(std::fs::read_to_string(path).map_err(|e| {
        CryptoError::Provider(format!("{BACKEND}: cannot read {variable}: {e}"))
    })?);
    let token = Zeroizing::new(raw.trim().to_owned());
    if token.is_empty() {
        return Err(CryptoError::Provider(format!(
            "{BACKEND}: the file {variable} names is empty"
        )));
    }
    Ok(token)
}

/// Reads `AccessKeyId`, `SecretAccessKey`, the session token under
/// `token_field`, and `Expiration` (RFC 3339 text or epoch seconds).
fn parse_credentials(
    body: &Value,
    token_field: &str,
) -> Result<(AwsCredentials, Option<SystemTime>), CryptoError> {
    let field = |name: &str| {
        body.get(name)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                CryptoError::Provider(format!(
                    "{BACKEND}: response lacks {name}"
                ))
            })
    };
    let credentials = AwsCredentials::new(
        field("AccessKeyId")?,
        field("SecretAccessKey")?,
        Some(field(token_field)?.to_owned()),
    );
    let expires = match body.get("Expiration") {
        Some(Value::String(text)) => {
            Some(parse_rfc3339(text).ok_or_else(|| {
                CryptoError::Provider(format!(
                    "{BACKEND}: Expiration is not a date"
                ))
            })?)
        }
        Some(Value::Number(n)) => n
            .as_f64()
            .filter(|s| s.is_finite() && *s >= 0.0)
            .map(|s| UNIX_EPOCH + Duration::from_secs_f64(s)),
        _ => None,
    };
    // Temporary credentials without an expiry would be cached forever;
    // treat them as short-lived instead.
    let expires =
        expires.or_else(|| Some(SystemTime::now() + REFRESH_MARGIN * 2));
    Ok((credentials, expires))
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.fff]Z` (or `+00:00`), the only form the
/// AWS credential endpoints return.
fn parse_rfc3339(text: &str) -> Option<SystemTime> {
    let text = text.trim();
    let body = text
        .strip_suffix('Z')
        .or_else(|| text.strip_suffix("+00:00"))?;
    let (date, time) = body.split_once('T')?;
    let mut d = date.split('-').map(str::parse::<i64>);
    let (year, month, day) =
        (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let time = time.split('.').next()?;
    let mut t = time.split(':').map(str::parse::<i64>);
    let (hour, minute, second) =
        (t.next()?.ok()?, t.next()?.ok()?, t.next()?.ok()?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3_600 + minute * 60 + second;
    u64::try_from(secs)
        .ok()
        .map(|s| UNIX_EPOCH + Duration::from_secs(s))
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `application/x-www-form-urlencoded` encoding.
fn form_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(byte, b'-' | b'_' | b'.' | b'~')
        {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars<'a>(
        pairs: &'a [(&'a str, &'a str)],
    ) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn the_chain_follows_the_sdk_order() -> Result<(), CryptoError> {
        let kind = |pairs: &[(&str, &str)]| {
            AwsCredentialProvider::from_vars(vars(pairs))
        };
        assert_eq!(
            kind(&[
                ("AWS_ACCESS_KEY_ID", "a"),
                ("AWS_SECRET_ACCESS_KEY", "b")
            ])?
            .kind(),
            "static"
        );
        assert_eq!(
            kind(&[
                ("AWS_WEB_IDENTITY_TOKEN_FILE", "/var/run/token"),
                ("AWS_ROLE_ARN", "arn:aws:iam::1:role/r"),
                ("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "/v2/creds"),
            ])?
            .kind(),
            "web-identity"
        );
        assert_eq!(
            kind(&[("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "/v2/creds")])?
                .kind(),
            "container"
        );
        assert_eq!(
            kind(&[
                (
                    "AWS_CONTAINER_CREDENTIALS_FULL_URI",
                    "http://169.254.170.23/v1/credentials"
                ),
                ("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE", "/var/run/eks"),
            ])?
            .kind(),
            "container"
        );
        assert_eq!(kind(&[])?.kind(), "imds");
        assert!(kind(&[("AWS_EC2_METADATA_DISABLED", "true")]).is_err());
        Ok(())
    }

    #[test]
    fn half_configured_sources_are_refused() {
        let refused = |pairs: &[(&str, &str)]| {
            AwsCredentialProvider::from_vars(vars(pairs))
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default()
        };
        assert!(
            refused(&[("AWS_ACCESS_KEY_ID", "a")])
                .contains("AWS_SECRET_ACCESS_KEY")
        );
        assert!(
            refused(&[("AWS_WEB_IDENTITY_TOKEN_FILE", "/t")])
                .contains("AWS_ROLE_ARN")
        );
        assert!(
            refused(&[("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "v2")])
                .contains("start with /")
        );
    }

    #[test]
    fn container_urls_must_not_leak_the_token() {
        for good in [
            "https://creds.example.com/x",
            "http://127.0.0.1:9911/creds",
            "http://localhost/creds",
            "http://169.254.170.2/v2/credentials/abc",
            "http://169.254.170.23/v1/credentials",
            "http://[fd00:ec2::23]/v1/credentials",
            "http://[::1]:80/creds",
        ] {
            assert!(check_container_url(good).is_ok(), "{good}");
        }
        for bad in [
            "http://evil.example.com/creds",
            "http://169.254.169.254/latest",
            "http://127.0.0.1.evil.com/x",
            "http://[fd00:ec2::24]/x",
            "ftp://127.0.0.1/x",
            "",
        ] {
            assert!(check_container_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn sts_uses_the_regional_endpoint() {
        assert_eq!(
            sts_endpoint(&vars(&[("AWS_REGION", "eu-west-1")])),
            "https://sts.eu-west-1.amazonaws.com"
        );
        assert_eq!(
            sts_endpoint(&vars(&[("AWS_DEFAULT_REGION", "us-east-2")])),
            "https://sts.us-east-2.amazonaws.com"
        );
        assert_eq!(sts_endpoint(&vars(&[])), "https://sts.amazonaws.com");
        assert_eq!(
            sts_endpoint(&vars(&[("AWS_REGION", "bad/region")])),
            "https://sts.amazonaws.com"
        );
    }

    #[test]
    fn expirations_parse() {
        let at = |s: &str| {
            parse_rfc3339(s).and_then(|t| {
                t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
            })
        };
        assert_eq!(at("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(at("2026-10-29T21:44:58Z"), Some(1_793_310_298));
        assert_eq!(at("2026-10-29T21:44:58.123Z"), Some(1_793_310_298));
        assert_eq!(at("2026-10-29T21:44:58+00:00"), Some(1_793_310_298));
        assert_eq!(at("2026-13-29T21:44:58Z"), None);
        assert_eq!(at("2026-10-29 21:44:58"), None);
        assert_eq!(at("garbage"), None);
    }

    #[test]
    fn credentials_never_show_in_debug_or_errors() {
        let body = serde_json::json!({
            "AccessKeyId": "ASIAEXAMPLE",
            "SecretAccessKey": "very-secret",
            "Token": "session-secret",
        });
        let (credentials, expires) = parse_credentials(&body, "Token")
            .unwrap_or_else(|_| (AwsCredentials::new("x", "y", None), None));
        let shown = format!("{credentials:?}");
        assert!(
            !shown.contains("very-secret") && !shown.contains("session-secret")
        );
        assert!(expires.is_some(), "temporary credentials always expire");
        let missing = parse_credentials(
            &serde_json::json!({"AccessKeyId": "a"}),
            "Token",
        )
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
        assert!(missing.contains("SecretAccessKey"));
    }

    #[test]
    fn form_encoding_escapes_reserved_characters() {
        assert_eq!(form_encode("a b&c=d/e+f"), "a%20b%26c%3Dd%2Fe%2Bf");
        assert_eq!(
            form_encode("arn:aws:iam::1:role/r"),
            "arn%3Aaws%3Aiam%3A%3A1%3Arole%2Fr"
        );
    }
}
