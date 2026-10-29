// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The shared error type.
//!
//! Crates keep their own precise error enums and convert into [`Error`] at
//! service boundaries. The [`ErrorKind`] decides the HTTP status, MCP error
//! code and retry policy; the message is safe to show to the caller, and the
//! source chain carries internal detail for logs and traces only.

use std::borrow::Cow;
use std::fmt;

use crate::id::IdParseError;
use crate::share::ShareTokenError;

/// What went wrong, in terms every caller can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The request was malformed or failed validation.
    InvalidInput,
    /// The caller is not signed in, or the credential is invalid or expired.
    Unauthenticated,
    /// The caller is signed in but not allowed to do this (deny by default).
    PermissionDenied,
    /// The object does not exist, or the caller may not know it exists.
    NotFound,
    /// The write conflicts with the current state (stale version, duplicate).
    Conflict,
    /// The caller exceeded a rate limit or quota.
    RateLimited,
    /// A dependency is down or overloaded; retrying later may succeed.
    Unavailable,
    /// The operation ran out of its time budget.
    Timeout,
    /// A bug or broken invariant; never caused by the caller.
    Internal,
}

impl ErrorKind {
    /// A stable `snake_case` code for APIs, logs and metrics labels.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_input",
            Self::Unauthenticated => "unauthenticated",
            Self::PermissionDenied => "permission_denied",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::RateLimited => "rate_limited",
            Self::Unavailable => "unavailable",
            Self::Timeout => "timeout",
            Self::Internal => "internal",
        }
    }

    /// Whether the same request may succeed if retried later unchanged.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::Unavailable | Self::Timeout)
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

type Source = Box<dyn std::error::Error + Send + Sync + 'static>;

/// An error with a kind, a caller-safe message and an optional source.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: Cow<'static, str>,
    source: Option<Source>,
}

/// A `Result` defaulting to the kernel [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// Creates an error. `message` must be safe to show to the caller.
    pub fn new(kind: ErrorKind, message: impl Into<Cow<'static, str>>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    /// Attaches the underlying cause, kept for logs and traces.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<Source>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Returns the kind.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the caller-safe message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|s| s as &(dyn std::error::Error + 'static))
    }
}

impl From<IdParseError> for Error {
    fn from(err: IdParseError) -> Self {
        Self::new(ErrorKind::InvalidInput, err.to_string()).with_source(err)
    }
}

impl From<ShareTokenError> for Error {
    fn from(err: ShareTokenError) -> Self {
        match err {
            // Never reveal why a presented token was rejected.
            ShareTokenError::Malformed | ShareTokenError::MalformedHash => {
                Self::new(ErrorKind::NotFound, "share link not found")
            }
            ShareTokenError::Random(_) => {
                Self::new(ErrorKind::Internal, "could not create share link").with_source(err)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn display_and_source() {
        let parse = "nope".parse::<crate::DocId>().err();
        let err = parse.map(Error::from);
        assert_eq!(err.as_ref().map(Error::kind), Some(ErrorKind::InvalidInput));
        assert_eq!(
            err.as_ref().map(ToString::to_string).as_deref(),
            Some("invalid_input: DocId must be 26 characters, got 4")
        );
        assert!(err.as_ref().and_then(|e| e.source()).is_some());
    }

    #[test]
    fn malformed_share_tokens_look_like_missing_links() {
        let err = Error::from(ShareTokenError::Malformed);
        assert_eq!(err.kind(), ErrorKind::NotFound);
        assert!(err.source().is_none());
    }

    #[test]
    fn retryable_kinds() {
        assert!(ErrorKind::Unavailable.is_retryable());
        assert!(!ErrorKind::Conflict.is_retryable());
        assert_eq!(ErrorKind::PermissionDenied.code(), "permission_denied");
    }
}
