// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The user's own model: settings, the [`ModelClient`] port and the HTTP
//! adapter for Anthropic and OpenAI-compatible endpoints.
//!
//! The API key arrives with each request and is never stored or logged
//! (D238): [`ApiKey`] has no `Serialize` and a redacted `Debug`.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;
use vellum_kernel::{Error, ErrorKind};
use vellum_security::ssrf::{SafeClient, SsrfError, UrlPolicy};

/// The Anthropic API version sent with every request.
pub const ANTHROPIC_VERSION: &str = "2026-10-21";

/// Longest model name accepted.
const MAX_MODEL_LEN: usize = 200;
/// Longest API key accepted.
const MAX_KEY_LEN: usize = 4096;
/// Longest provider error message passed on to the user.
const MAX_ERROR_LEN: usize = 300;

/// The wire format a model endpoint speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    /// The Anthropic Messages API (`POST {base}/v1/messages`).
    Anthropic,
    /// An OpenAI-style chat completions API (`POST {base}/chat/completions`):
    /// `OpenAI`, and self-hosted servers such as Ollama, LM Studio or vLLM.
    OpenaiCompatible,
}

impl Provider {
    /// The endpoint used when the settings name none.
    #[must_use]
    pub const fn default_base_url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com",
            Self::OpenaiCompatible => "https://api.openai.com/v1",
        }
    }
}

/// A model provider's API key. It is accepted from the request and passed
/// to the provider; it is never serialised, stored or printed.
#[derive(Clone, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key (for tests and adapters).
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The key itself, for the outbound header only.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

/// Which model to call and how: sent by the browser with every request.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ModelSettings {
    /// The wire format.
    pub provider: Provider,
    /// The endpoint; the provider's public API when absent.
    #[serde(default)]
    pub base_url: Option<String>,
    /// The model name, such as `claude-opus-5-5`.
    pub model: String,
    /// The user's own API key. Optional only for an endpoint an admin
    /// configured that needs none (a self-hosted model).
    #[serde(default)]
    pub api_key: Option<ApiKey>,
}

impl ModelSettings {
    /// The endpoint base URL, without a trailing slash.
    #[must_use]
    pub fn base(&self) -> String {
        self.base_url
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| self.provider.default_base_url())
            .trim_end_matches('/')
            .to_owned()
    }

    /// Checks the settings before any I/O.
    ///
    /// # Errors
    ///
    /// `invalid_input` naming the field that is wrong.
    pub fn validate(&self) -> Result<(), Error> {
        let model = self.model.trim();
        if model.is_empty()
            || model.len() > MAX_MODEL_LEN
            || !model.chars().all(|c| c.is_ascii_graphic())
        {
            return Err(invalid(
                "model: give the model's name, such as claude-opus-5-5",
            ));
        }
        if let Some(key) = &self.api_key {
            let key = key.expose();
            if key.is_empty()
                || key.len() > MAX_KEY_LEN
                || !key.chars().all(|c| c.is_ascii_graphic())
            {
                return Err(invalid(
                    "api_key: the key has spaces or characters a key never has",
                ));
            }
        }
        let url = Url::parse(&self.base()).map_err(|_| invalid("base_url: not an absolute URL"))?;
        // The request URL is built by appending the API path, so a query,
        // fragment or sign-in part would change where it goes (RT-038).
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid(
                "base_url: give the endpoint as http(s)://host[:port]/path, without a query",
            ));
        }
        Ok(())
    }
}

fn invalid(message: &str) -> Error {
    let message = message.to_owned();
    Error::new(ErrorKind::InvalidInput, message)
}

/// One request to a model: a system prompt and one user message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    /// Standing instructions.
    pub system: String,
    /// The task, with any quoted sources.
    pub user: String,
    /// The most tokens the model may produce.
    pub max_tokens: u32,
}

/// What the model answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The answer's text.
    pub text: String,
    /// The model stopped at `max_tokens`.
    pub truncated: bool,
}

/// Why a model call failed.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// The endpoint is not one Vellum may reach.
    #[error(
        "Vellum can't reach {0}: use the provider's public https address, or ask an admin to allow a self-hosted endpoint"
    )]
    Blocked(String),
    /// The provider refused the key.
    #[error("the provider refused the key ({0})")]
    Key(String),
    /// The provider is rate limiting the key.
    #[error("the provider is rate limiting this key; try again shortly")]
    RateLimited,
    /// The provider rejected the request (unknown model, too long).
    #[error("the provider rejected the request: {0}")]
    Rejected(String),
    /// The provider could not be reached or failed.
    #[error("the provider is unavailable: {0}")]
    Unavailable(String),
    /// The model declined to answer.
    #[error("the model declined to answer this request")]
    Declined,
    /// The provider answered with something unreadable.
    #[error("the provider's answer could not be read: {0}")]
    Malformed(String),
}

impl From<ModelError> for Error {
    fn from(e: ModelError) -> Self {
        let kind = match &e {
            ModelError::Blocked(_) | ModelError::Rejected(_) | ModelError::Declined => {
                ErrorKind::InvalidInput
            }
            // The user's provider key, not their Vellum session: a 401
            // would sign them out of Vellum.
            ModelError::Key(_) => ErrorKind::PermissionDenied,
            ModelError::RateLimited => ErrorKind::RateLimited,
            ModelError::Unavailable(_) | ModelError::Malformed(_) => ErrorKind::Unavailable,
        };
        Self::new(kind, e.to_string())
    }
}

/// Port: calls the user's model.
#[async_trait]
pub trait ModelClient: Send + Sync {
    /// Sends one prompt and returns the answer.
    async fn complete(
        &self,
        settings: &ModelSettings,
        prompt: &Prompt,
    ) -> Result<Completion, ModelError>;
}

/// A request ready to send: URL, headers (with the key) and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireRequest {
    /// Where to POST.
    pub url: String,
    /// Extra headers.
    pub headers: Vec<(&'static str, String)>,
    /// The JSON body.
    pub body: Value,
}

/// Builds the provider request for `prompt`.
#[must_use]
pub fn wire_request(settings: &ModelSettings, prompt: &Prompt) -> WireRequest {
    let base = settings.base();
    let key = settings.api_key.as_ref().map(ApiKey::expose);
    match settings.provider {
        Provider::Anthropic => WireRequest {
            url: format!("{base}/v1/messages"),
            headers: key
                .map(|k| ("x-api-key", k.to_owned()))
                .into_iter()
                .chain([("anthropic-version", ANTHROPIC_VERSION.to_owned())])
                .collect(),
            body: json!({
                "model": settings.model.trim(),
                "max_tokens": prompt.max_tokens,
                "system": prompt.system,
                "messages": [{ "role": "user", "content": prompt.user }],
            }),
        },
        Provider::OpenaiCompatible => WireRequest {
            url: format!("{base}/chat/completions"),
            headers: key
                .map(|k| ("authorization", format!("Bearer {k}")))
                .into_iter()
                .collect(),
            body: json!({
                "model": settings.model.trim(),
                "max_tokens": prompt.max_tokens,
                "messages": [
                    { "role": "system", "content": prompt.system },
                    { "role": "user", "content": prompt.user },
                ],
            }),
        },
    }
}

fn provider_message(body: &Value) -> Option<String> {
    body.pointer("/error/message")
        .or_else(|| body.get("error"))
        .or_else(|| body.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn shorten(text: &str, key: Option<&str>) -> String {
    let mut text = text.to_owned();
    if let Some(key) = key.filter(|k| k.len() >= 4) {
        text = text.replace(key, "…");
    }
    let cut: String = text.chars().take(MAX_ERROR_LEN).collect();
    if cut.len() < text.len() {
        format!("{cut}…")
    } else {
        cut
    }
}

/// Reads a provider's response.
///
/// # Errors
///
/// The [`ModelError`] that the status or body means.
pub fn read_response(
    provider: Provider,
    status: u16,
    body: &[u8],
    key: Option<&str>,
) -> Result<Completion, ModelError> {
    let json: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    if !(200..300).contains(&status) {
        let message = shorten(
            &provider_message(&json).unwrap_or_else(|| format!("HTTP {status}")),
            key,
        );
        return Err(match status {
            401 | 403 => ModelError::Key(message),
            429 => ModelError::RateLimited,
            400..=499 => ModelError::Rejected(message),
            _ => ModelError::Unavailable(message),
        });
    }
    let (text, stop) = match provider {
        Provider::Anthropic => {
            let text = json.get("content").and_then(Value::as_array).map(|blocks| {
                blocks
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<String>()
            });
            (text, json.get("stop_reason").and_then(Value::as_str))
        }
        Provider::OpenaiCompatible => {
            let choice = json.pointer("/choices/0");
            (
                choice
                    .and_then(|c| c.pointer("/message/content"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                choice
                    .and_then(|c| c.get("finish_reason"))
                    .and_then(Value::as_str),
            )
        }
    };
    if matches!(stop, Some("refusal" | "content_filter")) {
        return Err(ModelError::Declined);
    }
    let text = text.ok_or_else(|| ModelError::Malformed("no text in the answer".into()))?;
    let truncated = matches!(stop, Some("max_tokens" | "length"));
    if text.trim().is_empty() {
        return Err(if truncated {
            ModelError::Rejected("the model ran out of tokens before answering".into())
        } else {
            ModelError::Malformed("the answer was empty".into())
        });
    }
    Ok(Completion { text, truncated })
}

/// The HTTP adapter: every call goes through the SSRF guard. Public
/// endpoints must use https; endpoints an admin listed (a self-hosted
/// model on the private network) may use http and private addresses.
#[derive(Debug, Clone)]
pub struct HttpModel {
    client: SafeClient,
    endpoints: Vec<Url>,
}

impl HttpModel {
    /// A client allowing the public internet plus `endpoints`
    /// (`VELLUM_AI_ENDPOINTS`), each an absolute base URL.
    ///
    /// # Errors
    ///
    /// `invalid_input` for an endpoint that is not a URL with a host, and
    /// `internal` if the TLS backend fails.
    pub fn new(endpoints: &[String], timeout: Duration) -> Result<Self, Error> {
        let endpoints = endpoints
            .iter()
            .map(|e| {
                Url::parse(e.trim().trim_end_matches('/'))
                    .ok()
                    .filter(|u| u.host_str().is_some())
                    .ok_or_else(|| invalid(&format!("VELLUM_AI_ENDPOINTS: {e} is not a URL")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut policy = UrlPolicy::default();
        policy.trusted_hosts = endpoints
            .iter()
            .filter_map(|u| u.host_str().map(str::to_owned))
            .collect();
        policy.timeout = timeout;
        policy.max_body_bytes = 4 * 1024 * 1024;
        let client = SafeClient::new(policy).map_err(|e| {
            Error::new(ErrorKind::Internal, "cannot build the model client").with_source(e)
        })?;
        Ok(Self { client, endpoints })
    }

    /// Whether `base` is one of the admin-listed endpoints: same scheme,
    /// host, port and path. Only the exact base an admin listed is
    /// trusted, never other paths on that host (RT-038), so a user cannot
    /// reach a self-hosted server's other APIs, such as Ollama's model
    /// pulls.
    #[must_use]
    pub fn is_listed(&self, base: &str) -> bool {
        let Ok(url) = Url::parse(base) else {
            return false;
        };
        self.endpoints.iter().any(|e| {
            e.scheme() == url.scheme()
                && e.host_str() == url.host_str()
                && e.port_or_known_default() == url.port_or_known_default()
                && url.path().trim_end_matches('/') == e.path().trim_end_matches('/')
                && url.query().is_none()
                && url.fragment().is_none()
                && url.username().is_empty()
                && url.password().is_none()
        })
    }
}

#[async_trait]
impl ModelClient for HttpModel {
    async fn complete(
        &self,
        settings: &ModelSettings,
        prompt: &Prompt,
    ) -> Result<Completion, ModelError> {
        settings
            .validate()
            .map_err(|e| ModelError::Blocked(e.message().to_owned()))?;
        let base = settings.base();
        let listed = self.is_listed(&base);
        if !listed && !base.starts_with("https://") {
            return Err(ModelError::Blocked(base));
        }
        let key = settings.api_key.as_ref().map(ApiKey::expose);
        if key.is_none() && !listed {
            return Err(ModelError::Key("no API key was given".into()));
        }
        let request = wire_request(settings, prompt);
        let headers: Vec<(&str, &str)> = request
            .headers
            .iter()
            .map(|(k, v)| (*k, v.as_str()))
            .collect();
        let body =
            serde_json::to_vec(&request.body).map_err(|e| ModelError::Malformed(e.to_string()))?;
        let started = std::time::Instant::now();
        let (status, bytes) = self
            .client
            .post_json(&request.url, &headers, body, listed)
            .await
            .map_err(|e| match e {
                SsrfError::Http(e) if e.is_timeout() => {
                    ModelError::Unavailable("the request timed out".into())
                }
                SsrfError::Http(_) => ModelError::Unavailable("the endpoint did not answer".into()),
                SsrfError::TooLarge(_) => ModelError::Malformed("the answer was too large".into()),
                other => ModelError::Blocked(format!("{base} ({other})")),
            })?;
        tracing::info!(
            provider = ?settings.provider,
            model = settings.model.trim(),
            status,
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "model call"
        );
        read_response(settings.provider, status, &bytes, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(provider: Provider) -> ModelSettings {
        ModelSettings {
            provider,
            base_url: None,
            model: "claude-opus-5-5".into(),
            api_key: Some(ApiKey::new("sk-test-123456")),
        }
    }

    fn prompt() -> Prompt {
        Prompt {
            system: "sys".into(),
            user: "hello".into(),
            max_tokens: 100,
        }
    }

    #[test]
    fn anthropic_request_shape() {
        let r = wire_request(&settings(Provider::Anthropic), &prompt());
        assert_eq!(r.url, "https://api.anthropic.com/v1/messages");
        assert!(r.headers.contains(&("x-api-key", "sk-test-123456".into())));
        assert!(
            r.headers
                .contains(&("anthropic-version", ANTHROPIC_VERSION.into()))
        );
        assert_eq!(r.body["system"], "sys");
        assert_eq!(r.body["messages"][0]["content"], "hello");
        assert!(r.body.get("temperature").is_none());
    }

    #[test]
    fn openai_request_shape() {
        let mut s = settings(Provider::OpenaiCompatible);
        s.base_url = Some("https://llm.northstar.dev/v1/".into());
        let r = wire_request(&s, &prompt());
        assert_eq!(r.url, "https://llm.northstar.dev/v1/chat/completions");
        assert_eq!(
            r.headers,
            vec![("authorization", "Bearer sk-test-123456".to_owned())]
        );
        assert_eq!(r.body["messages"][0]["role"], "system");
    }

    #[test]
    fn reads_answers_and_errors() {
        let ok = br#"{"content":[{"type":"thinking","thinking":""},{"type":"text","text":"Hi"}],"stop_reason":"end_turn"}"#;
        assert_eq!(
            read_response(Provider::Anthropic, 200, ok, None).ok(),
            Some(Completion {
                text: "Hi".into(),
                truncated: false
            })
        );
        let refusal = br#"{"content":[],"stop_reason":"refusal"}"#;
        assert!(matches!(
            read_response(Provider::Anthropic, 200, refusal, None),
            Err(ModelError::Declined)
        ));
        let oa = br#"{"choices":[{"message":{"content":"Yo"},"finish_reason":"length"}]}"#;
        assert_eq!(
            read_response(Provider::OpenaiCompatible, 200, oa, None)
                .ok()
                .map(|c| c.truncated),
            Some(true)
        );
        let bad = br#"{"error":{"message":"invalid key sk-test-123456"}}"#;
        let err = read_response(Provider::Anthropic, 401, bad, Some("sk-test-123456"));
        assert!(
            matches!(&err, Err(ModelError::Key(m)) if !m.contains("sk-test")),
            "{err:?}"
        );
        assert!(matches!(
            read_response(Provider::Anthropic, 429, b"{}", None),
            Err(ModelError::RateLimited)
        ));
    }

    #[test]
    fn keys_never_print() {
        let s = settings(Provider::Anthropic);
        assert!(!format!("{s:?}").contains("sk-test"));
    }

    #[test]
    fn validation_names_the_field() {
        let mut s = settings(Provider::Anthropic);
        s.model = "bad model".into();
        assert!(s.validate().is_err());
        let mut s = settings(Provider::Anthropic);
        s.api_key = Some(ApiKey::new("sk test"));
        assert!(s.validate().is_err());
        assert!(settings(Provider::Anthropic).validate().is_ok());
    }

    #[tokio::test]
    async fn plain_http_and_private_hosts_need_an_admin_listing() -> Result<(), Error> {
        let model = HttpModel::new(&["http://ollama:11434/v1".into()], Duration::from_secs(5))?;
        assert!(model.is_listed("http://ollama:11434/v1"));
        assert!(!model.is_listed("http://ollama:11435/v1"));
        let mut s = settings(Provider::OpenaiCompatible);
        s.base_url = Some("http://169.254.169.254/latest".into());
        assert!(matches!(
            model.complete(&s, &prompt()).await,
            Err(ModelError::Blocked(_))
        ));
        s.base_url = Some("https://127.0.0.1/v1".into());
        assert!(matches!(
            model.complete(&s, &prompt()).await,
            Err(ModelError::Blocked(_))
        ));
        Ok(())
    }

    /// RT-038: a listed endpoint trusts its exact base only. Other paths on
    /// the host (Ollama's `/api/pull`, `/api/push`) stay unreachable, and a
    /// query or fragment cannot cut off the API path that is appended.
    #[tokio::test]
    async fn a_listing_trusts_its_exact_base_only() -> Result<(), Error> {
        let model = HttpModel::new(
            &["http://ollama:11434".into(), "http://vllm:8000/v1/".into()],
            Duration::from_secs(5),
        )?;
        assert!(model.is_listed("http://ollama:11434"));
        assert!(model.is_listed("http://vllm:8000/v1"));
        for base in [
            "http://ollama:11434/api",
            "http://ollama:11434/api/pull?",
            "http://vllm:8000/v1x",
            "http://vllm:8000/v1/../api",
            "http://vllm:8000/v1/admin",
            "http://vllm:8000/v1?a=b",
            "http://vllm:8000/v1#x",
            "http://u:p@vllm:8000/v1",
        ] {
            assert!(!model.is_listed(base), "{base}");
            let mut s = settings(Provider::OpenaiCompatible);
            s.base_url = Some(base.into());
            assert!(
                matches!(
                    model.complete(&s, &prompt()).await,
                    Err(ModelError::Blocked(_))
                ),
                "{base}"
            );
        }
        for base in [
            "https://api.openai.com/v1?x=",
            "https://api.openai.com/v1#frag",
            "https://user@api.openai.com/v1",
            "ftp://api.openai.com/v1",
        ] {
            let mut s = settings(Provider::OpenaiCompatible);
            s.base_url = Some(base.into());
            assert!(s.validate().is_err(), "{base}");
        }
        Ok(())
    }
}
