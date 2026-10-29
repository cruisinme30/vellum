// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Opt-in anonymous usage telemetry (brief §3).
//!
//! Off by default. When an admin turns it on, Vellum sends a small report
//! every `interval_hours`: the version, platform, deployment profile and
//! rough size buckets. It never contains names, ids other than the random
//! installation id, content, IP addresses or exact counts. The admin
//! console shows the exact JSON that would be sent ([`build_report`]), and
//! every report sent is audited.

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_kernel::Result;

use crate::settings::TelemetrySettings;

/// The report format version.
pub const REPORT_SCHEMA: u32 = 1;

/// Facts the server knows about itself, gathered by the caller.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageFacts {
    /// Active people.
    pub users: u64,
    /// Spaces.
    pub spaces: u64,
    /// Documents, including rows and issues.
    pub documents: u64,
    /// `single`, `docker`, `compose` or `helm`.
    pub profile: String,
    /// `sqlite` or `postgres`.
    pub database: String,
    /// Optional features turned on, such as `kroki`, `redis`, `e2e_spaces`.
    pub features: Vec<String>,
}

/// Exactly what is sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageReport {
    /// [`REPORT_SCHEMA`].
    pub schema: u32,
    /// The random installation id.
    pub install_id: String,
    /// The Vellum version.
    pub version: String,
    /// `linux`, `macos` or `windows`.
    pub os: String,
    /// `x86_64` or `aarch64`.
    pub arch: String,
    /// The deployment profile.
    pub profile: String,
    /// The database.
    pub database: String,
    /// Size bucket of active people.
    pub users: String,
    /// Size bucket of spaces.
    pub spaces: String,
    /// Size bucket of documents.
    pub documents: String,
    /// Optional features turned on, sorted.
    pub features: Vec<String>,
}

/// Rounds a count to a coarse bucket so exact sizes are never sent.
#[must_use]
pub const fn bucket(n: u64) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2..=10 => "2-10",
        11..=50 => "11-50",
        51..=250 => "51-250",
        251..=1_000 => "251-1000",
        1_001..=10_000 => "1001-10000",
        10_001..=100_000 => "10001-100000",
        _ => "100001+",
    }
}

fn token(s: &str, allowed: &[&str]) -> String {
    if allowed.contains(&s) {
        s.to_owned()
    } else {
        "other".to_owned()
    }
}

const FEATURES: &[&str] = &[
    "kroki",
    "redis",
    "meilisearch",
    "gotenberg",
    "e2e_spaces",
    "git_backend",
    "s3",
    "google_drive",
    "dropbox",
    "onedrive",
    "sso_oidc",
    "sso_saml",
    "mcp",
    "desktop",
    "embeddings",
];

/// Builds the report for `facts`. Free text is never copied: profile,
/// database and features are mapped onto fixed vocabularies.
#[must_use]
pub fn build_report(
    settings: &TelemetrySettings,
    facts: &UsageFacts,
    version: &str,
) -> UsageReport {
    let mut features: Vec<String> = facts
        .features
        .iter()
        .filter(|f| FEATURES.contains(&f.as_str()))
        .cloned()
        .collect();
    features.sort();
    features.dedup();
    UsageReport {
        schema: REPORT_SCHEMA,
        install_id: settings.install_id.clone(),
        version: version.chars().take(32).collect(),
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
        profile: token(&facts.profile, &["single", "docker", "compose", "helm"]),
        database: token(&facts.database, &["sqlite", "postgres"]),
        users: bucket(facts.users).to_owned(),
        spaces: bucket(facts.spaces).to_owned(),
        documents: bucket(facts.documents).to_owned(),
        features,
    }
}

/// Whether a report is due: telemetry is on, an endpoint is set, and the
/// interval has passed since `last_sent_ms` (Unix milliseconds).
#[must_use]
pub fn is_due(settings: &TelemetrySettings, last_sent_ms: Option<i64>, now_ms: i64) -> bool {
    let interval = i64::from(settings.interval_hours) * 3_600_000;
    settings.enabled
        && settings.endpoint.is_some()
        && last_sent_ms.is_none_or(|last| now_ms - last >= interval)
}

/// Delivers reports.
#[async_trait]
pub trait UsageTransport: Send + Sync {
    /// Posts `report` as JSON to `endpoint`.
    async fn send(&self, endpoint: &str, report: &UsageReport) -> Result<()>;
}

/// Sends reports over HTTPS with a short timeout. Only `https://`
/// endpoints are accepted, and redirects are not followed, so a report
/// can never be bounced to another host.
#[derive(Debug, Clone)]
pub struct HttpUsageTransport {
    client: reqwest::Client,
}

impl HttpUsageTransport {
    /// A transport with a 10-second timeout.
    ///
    /// # Errors
    ///
    /// Returns an internal error if the TLS client cannot be built.
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("vellum/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| {
                vellum_kernel::Error::new(
                    vellum_kernel::ErrorKind::Internal,
                    "cannot build the HTTP client",
                )
                .with_source(e)
            })?;
        Ok(Self { client })
    }
}

#[async_trait]
impl UsageTransport for HttpUsageTransport {
    #[tracing::instrument(name = "telemetry.send", skip_all)]
    async fn send(&self, endpoint: &str, report: &UsageReport) -> Result<()> {
        use vellum_kernel::{Error, ErrorKind};
        if !endpoint.starts_with("https://") {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "telemetry endpoint must use https",
            ));
        }
        let body = serde_json::to_vec(report).map_err(|e| {
            Error::new(ErrorKind::Internal, "cannot encode the report").with_source(e)
        })?;
        let response = self
            .client
            .post(endpoint)
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| {
                Error::new(
                    ErrorKind::Unavailable,
                    "cannot reach the telemetry endpoint",
                )
                .with_source(e)
            })?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::Unavailable,
                format!("telemetry endpoint answered {}", response.status().as_u16()),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> UsageFacts {
        UsageFacts {
            users: 12,
            spaces: 3,
            documents: 4_000,
            profile: "compose".into(),
            database: "postgres; DROP".into(),
            features: vec![
                "redis".into(),
                "kroki".into(),
                "secret-name".into(),
                "redis".into(),
            ],
        }
    }

    #[test]
    fn buckets() {
        let cases = [
            (0, "0"),
            (1, "1"),
            (7, "2-10"),
            (50, "11-50"),
            (51, "51-250"),
            (999, "251-1000"),
            (5_000, "1001-10000"),
            (50_000, "10001-100000"),
            (u64::MAX, "100001+"),
        ];
        for (n, b) in cases {
            assert_eq!(bucket(n), b);
        }
    }

    #[test]
    fn reports_hold_no_free_text() {
        let settings = TelemetrySettings::default();
        let r = build_report(&settings, &facts(), "1.0.0");
        assert_eq!(r.install_id, settings.install_id);
        assert_eq!(r.users, "11-50");
        assert_eq!(r.documents, "1001-10000");
        assert_eq!(r.profile, "compose");
        assert_eq!(r.database, "other");
        assert_eq!(r.features, vec!["kroki", "redis"]);
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("secret-name"));
        assert!(!json.contains("DROP"));
    }

    #[tokio::test]
    async fn http_transport_refuses_plain_http_and_reports_failures() {
        let t = HttpUsageTransport::new().unwrap();
        let r = build_report(&TelemetrySettings::default(), &facts(), "1");
        let err = t.send("http://example.test", &r).await.unwrap_err();
        assert_eq!(err.kind(), vellum_kernel::ErrorKind::InvalidInput);
        // Nothing listens on port 9 locally.
        let err = t.send("https://127.0.0.1:9/report", &r).await.unwrap_err();
        assert_eq!(err.kind(), vellum_kernel::ErrorKind::Unavailable);
    }

    #[test]
    fn due_only_when_enabled_with_endpoint() {
        let mut s = TelemetrySettings::default();
        assert!(!is_due(&s, None, 0));
        s.enabled = true;
        assert!(!is_due(&s, None, 0));
        s.endpoint = Some("https://telemetry.example".into());
        assert!(is_due(&s, None, 0));
        let week = 168 * 3_600_000;
        assert!(!is_due(&s, Some(0), week - 1));
        assert!(is_due(&s, Some(0), week));
    }
}
