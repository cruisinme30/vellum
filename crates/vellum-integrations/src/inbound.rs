// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Shared errors for inbound webhooks (GitHub, GitLab, Slack, Sentry).
//!
//! Receivers verify the provider's signature over the raw body before
//! parsing anything, and answer 401 for [`InboundError::Unauthenticated`],
//! 400 for [`InboundError::Malformed`], and 202 for everything else, so a
//! provider never retries an event Vellum chose to ignore.

/// Why an inbound request was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InboundError {
    /// The signature or token is missing, wrong or stale.
    #[error("unauthenticated webhook")]
    Unauthenticated,
    /// The body is not what the provider documents.
    #[error("malformed payload: {0}")]
    Malformed(String),
}

impl InboundError {
    /// A malformed-payload error from anything printable.
    pub fn malformed(reason: impl std::fmt::Display) -> Self {
        Self::Malformed(reason.to_string())
    }

    /// The HTTP status the receiver answers.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::Unauthenticated => 401,
            Self::Malformed(_) => 400,
        }
    }
}

/// Reads a string field from JSON, or the empty string.
pub fn text<'a>(value: &'a serde_json::Value, pointer: &str) -> &'a str {
    value
        .pointer(pointer)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
}
