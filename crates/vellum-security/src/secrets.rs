// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Secret scanning on save (R-SE-6).
//!
//! [`scan`] looks for credentials in document text: provider tokens with
//! a recognisable prefix, private-key blocks, JSON Web Tokens, and
//! high-entropy values assigned to names such as `api_key` or `password`.
//! The editor shows a warning banner for each [`Finding`]; an admin can
//! set [`SecretPolicy::Block`] to refuse the save instead.
//!
//! Findings never carry the secret itself, only its position and a
//! redacted preview, so they are safe to log and return to clients. A
//! line containing `vellum:allow-secret` is skipped, for documentation
//! that shows example tokens on purpose.

use std::ops::Range;

/// The marker that exempts a line from scanning.
pub const ALLOW_MARKER: &str = "vellum:allow-secret";

/// The kind of credential a [`Finding`] looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Rule {
    /// AWS access key ID.
    AwsAccessKey,
    /// `GitHub` personal, `OAuth`, app or refresh token.
    GitHubToken,
    /// `GitLab` personal access token.
    GitLabToken,
    /// Slack bot, user or app token.
    SlackToken,
    /// Stripe live secret or restricted key.
    StripeKey,
    /// Google API key.
    GoogleApiKey,
    /// Anthropic API key.
    AnthropicKey,
    /// `OpenAI` project or service-account key.
    OpenAiKey,
    /// npm access token.
    NpmToken,
    /// PEM private key block.
    PrivateKey,
    /// JSON Web Token.
    Jwt,
    /// High-entropy value assigned to a secret-sounding name.
    Assignment,
}

impl Rule {
    /// A stable identifier for APIs, logs and the admin allow-list.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::AwsAccessKey => "aws-access-key",
            Self::GitHubToken => "github-token",
            Self::GitLabToken => "gitlab-token",
            Self::SlackToken => "slack-token",
            Self::StripeKey => "stripe-key",
            Self::GoogleApiKey => "google-api-key",
            Self::AnthropicKey => "anthropic-key",
            Self::OpenAiKey => "openai-key",
            Self::NpmToken => "npm-token",
            Self::PrivateKey => "private-key",
            Self::Jwt => "jwt",
            Self::Assignment => "secret-assignment",
        }
    }
}

/// A likely secret in a document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// What it looks like.
    pub rule: Rule,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column, in characters.
    pub column: usize,
    /// Byte range of the secret in the scanned text.
    pub range: Range<usize>,
    /// The first four characters and the length, for display.
    pub redacted: String,
}

/// What happens when a save contains secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum SecretPolicy {
    /// Do not scan.
    Off,
    /// Save, and show a warning for each finding.
    #[default]
    Warn,
    /// Refuse the save.
    Block,
}

/// A save refused by [`SecretPolicy::Block`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("document contains {} likely secret(s)", findings.len())]
pub struct SecretsFound {
    /// What was found.
    pub findings: Vec<Finding>,
}

impl SecretPolicy {
    /// Scans `text` under this policy.
    ///
    /// # Errors
    ///
    /// Returns [`SecretsFound`] when the policy is [`SecretPolicy::Block`]
    /// and the text contains a secret.
    pub fn check(self, text: &str) -> Result<Vec<Finding>, SecretsFound> {
        let findings = match self {
            Self::Off => return Ok(Vec::new()),
            Self::Warn | Self::Block => scan(text),
        };
        if self == Self::Block && !findings.is_empty() {
            return Err(SecretsFound { findings });
        }
        Ok(findings)
    }
}

/// Which characters a token body may contain.
#[derive(Clone, Copy)]
enum Chars {
    UpperDigit,
    Alnum,
    AlnumUnderscore,
    AlnumDash,
    AlnumDashUnderscore,
}

impl Chars {
    const fn allows(self, c: u8) -> bool {
        match self {
            Self::UpperDigit => c.is_ascii_uppercase() || c.is_ascii_digit(),
            Self::Alnum => c.is_ascii_alphanumeric(),
            Self::AlnumUnderscore => c.is_ascii_alphanumeric() || c == b'_',
            Self::AlnumDash => c.is_ascii_alphanumeric() || c == b'-',
            Self::AlnumDashUnderscore => c.is_ascii_alphanumeric() || c == b'-' || c == b'_',
        }
    }
}

/// A token with a fixed prefix and a body of `min..=max` characters.
struct Prefixed {
    rule: Rule,
    prefixes: &'static [&'static str],
    chars: Chars,
    min: usize,
    max: usize,
}

const PREFIXED: &[Prefixed] = &[
    Prefixed {
        rule: Rule::AwsAccessKey,
        prefixes: &["AKIA", "ASIA"],
        chars: Chars::UpperDigit,
        min: 16,
        max: 16,
    },
    Prefixed {
        rule: Rule::GitHubToken,
        prefixes: &["ghp_", "gho_", "ghu_", "ghs_", "ghr_"],
        chars: Chars::Alnum,
        min: 36,
        max: 255,
    },
    Prefixed {
        rule: Rule::GitHubToken,
        prefixes: &["github_pat_"],
        chars: Chars::AlnumUnderscore,
        min: 50,
        max: 255,
    },
    Prefixed {
        rule: Rule::GitLabToken,
        prefixes: &["glpat-"],
        chars: Chars::AlnumDashUnderscore,
        min: 20,
        max: 64,
    },
    Prefixed {
        rule: Rule::SlackToken,
        prefixes: &["xoxb-", "xoxp-", "xoxa-", "xoxr-", "xoxs-", "xapp-"],
        chars: Chars::AlnumDash,
        min: 10,
        max: 255,
    },
    Prefixed {
        rule: Rule::StripeKey,
        prefixes: &["sk_live_", "rk_live_"],
        chars: Chars::Alnum,
        min: 16,
        max: 255,
    },
    Prefixed {
        rule: Rule::GoogleApiKey,
        prefixes: &["AIza"],
        chars: Chars::AlnumDashUnderscore,
        min: 35,
        max: 35,
    },
    Prefixed {
        rule: Rule::AnthropicKey,
        prefixes: &["sk-ant-"],
        chars: Chars::AlnumDashUnderscore,
        min: 32,
        max: 255,
    },
    Prefixed {
        rule: Rule::OpenAiKey,
        prefixes: &["sk-proj-", "sk-svcacct-"],
        chars: Chars::AlnumDashUnderscore,
        min: 20,
        max: 255,
    },
    Prefixed {
        rule: Rule::NpmToken,
        prefixes: &["npm_"],
        chars: Chars::Alnum,
        min: 36,
        max: 36,
    },
];

/// Names whose assigned values are treated as secrets.
const SECRET_NAMES: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "access_key",
    "secret_key",
    "client_secret",
    "secret",
    "password",
    "passwd",
    "token",
    "auth_token",
    "private_key",
];

/// Values that are obviously placeholders.
const PLACEHOLDERS: &[&str] = &[
    "example",
    "xxxx",
    "changeme",
    "your",
    "dummy",
    "redacted",
    "placeholder",
    "<",
    "${",
    "{{",
];

/// Finds likely secrets in `text`.
#[must_use]
pub fn scan(text: &str) -> Vec<Finding> {
    let span = tracing::info_span!(
        "secrets.scan",
        bytes = text.len(),
        findings = tracing::field::Empty
    );
    let _entered = span.enter();
    let mut findings = Vec::new();
    let mut offset = 0;
    for (index, line) in text.split_inclusive('\n').enumerate() {
        if !line.contains(ALLOW_MARKER) {
            let start = findings.len();
            scan_prefixed(line, &mut findings);
            scan_private_key(line, &mut findings);
            scan_jwt(line, &mut findings);
            scan_assignment(line, &mut findings);
            for finding in findings.get_mut(start..).unwrap_or_default() {
                finding.line = index + 1;
                finding.column = line
                    .get(..finding.range.start)
                    .map_or(1, |head| head.chars().count() + 1);
                finding.range = offset + finding.range.start..offset + finding.range.end;
            }
        }
        offset += line.len();
    }
    findings.sort_by_key(|f| f.range.start);
    findings.dedup_by(|later, earlier| later.range.start < earlier.range.end);
    let _span = span.record("findings", findings.len());
    if !findings.is_empty() {
        tracing::info!(
            monotonic_counter.vellum.secrets.found = findings.len() as u64,
            "secrets found on save"
        );
    }
    findings
}

fn push(findings: &mut Vec<Finding>, rule: Rule, line: &str, range: Range<usize>) {
    let secret = line.get(range.clone()).unwrap_or_default();
    let head: String = secret.chars().take(4).collect();
    findings.push(Finding {
        rule,
        line: 0,
        column: 0,
        redacted: format!("{head}… ({} chars)", secret.chars().count()),
        range,
    });
}

/// Length of the run of `allowed` bytes starting at `from`.
fn run(bytes: &[u8], from: usize, allowed: impl Fn(u8) -> bool) -> usize {
    bytes
        .get(from..)
        .unwrap_or_default()
        .iter()
        .take_while(|&&c| allowed(c))
        .count()
}

const fn word_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

fn scan_prefixed(line: &str, findings: &mut Vec<Finding>) {
    let bytes = line.as_bytes();
    for rule in PREFIXED {
        for prefix in rule.prefixes {
            for (at, _) in line.match_indices(prefix) {
                let preceded = at
                    .checked_sub(1)
                    .and_then(|i| bytes.get(i))
                    .is_some_and(|&c| word_char(c));
                if preceded {
                    continue;
                }
                let body_start = at + prefix.len();
                let body = run(bytes, body_start, |c| rule.chars.allows(c));
                if (rule.min..=rule.max).contains(&body) {
                    push(findings, rule.rule, line, at..body_start + body);
                }
            }
        }
    }
}

fn scan_private_key(line: &str, findings: &mut Vec<Finding>) {
    for (at, _) in line.match_indices("-----BEGIN ") {
        let rest = line.get(at..).unwrap_or_default();
        if let Some(end) = rest
            .find("-----\n")
            .or_else(|| rest.trim_end().strip_suffix("-----").map(str::len))
            && rest
                .get(..end)
                .is_some_and(|header| header.ends_with("PRIVATE KEY"))
        {
            push(findings, Rule::PrivateKey, line, at..at + end + 5);
        }
    }
}

fn scan_jwt(line: &str, findings: &mut Vec<Finding>) {
    let bytes = line.as_bytes();
    let b64 = |c: u8| c.is_ascii_alphanumeric() || c == b'-' || c == b'_';
    for (at, _) in line.match_indices("eyJ") {
        if at
            .checked_sub(1)
            .and_then(|i| bytes.get(i))
            .is_some_and(|&c| b64(c))
        {
            continue;
        }
        let mut end = at;
        let mut parts = 0;
        loop {
            let len = run(bytes, end, b64);
            if len < 10 {
                break;
            }
            end += len;
            parts += 1;
            if parts == 3 || bytes.get(end) != Some(&b'.') {
                break;
            }
            end += 1;
        }
        let second_is_json = line
            .get(at..end)
            .and_then(|t| t.split('.').nth(1))
            .is_some_and(|p| p.starts_with("eyJ"));
        if parts == 3 && second_is_json {
            push(findings, Rule::Jwt, line, at..end);
        }
    }
}

fn scan_assignment(line: &str, findings: &mut Vec<Finding>) {
    let lower = line.to_ascii_lowercase();
    let bytes = line.as_bytes();
    for name in SECRET_NAMES {
        for (at, _) in lower.match_indices(name) {
            // The name may end a longer identifier (`db_password`) but must
            // not run into more letters (`tokens`).
            let after = at + name.len();
            if bytes.get(after).is_some_and(|&c| c.is_ascii_alphanumeric()) {
                continue;
            }
            let mut i = after;
            i += run(bytes, i, |c| {
                c == b'"' || c == b'\'' || c == b' ' || c == b'\t'
            });
            if !matches!(bytes.get(i), Some(b'=' | b':')) {
                continue;
            }
            i += 1;
            i += run(bytes, i, |c| {
                c == b' ' || c == b'\t' || c == b'"' || c == b'\'' || c == b'`'
            });
            let len = run(bytes, i, |c| {
                c.is_ascii_alphanumeric()
                    || matches!(
                        c,
                        b'_' | b'-'
                            | b'+'
                            | b'/'
                            | b'='
                            | b'.'
                            | b'!'
                            | b'@'
                            | b'#'
                            | b'$'
                            | b'%'
                            | b'^'
                            | b'&'
                            | b'*'
                    )
            });
            let value = line.get(i..i + len).unwrap_or_default();
            if len >= 16 && entropy(value) >= 3.5 && !is_placeholder(value) {
                push(findings, Rule::Assignment, line, i..i + len);
            }
        }
    }
}

fn is_placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    PLACEHOLDERS.iter().any(|p| lower.contains(p))
}

/// Shannon entropy in bits per character.
#[allow(clippy::cast_precision_loss)]
fn entropy(value: &str) -> f64 {
    let mut counts = [0_u32; 256];
    for &b in value.as_bytes() {
        if let Some(n) = counts.get_mut(usize::from(b)) {
            *n += 1;
        }
    }
    let len = value.len() as f64;
    counts
        .iter()
        .filter(|&&n| n > 0)
        .map(|&n| {
            let p = f64::from(n) / len;
            -p * p.log2()
        })
        .sum()
}
