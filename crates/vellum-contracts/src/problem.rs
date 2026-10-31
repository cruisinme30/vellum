// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The error body returned by every Vellum API.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_kernel::{Error, ErrorKind, RequestId};

/// The JSON body of every error response, REST and GraphQL alike
/// (`extensions` in GraphQL). Clients show `message` and offer
/// `request_id` as the "copy diagnostic" value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ApiError {
    /// A stable `snake_case` code: `invalid_input`, `unauthenticated`,
    /// `permission_denied`, `not_found`, `conflict`, `rate_limited`,
    /// `unavailable`, `timeout` or `internal`.
    pub code: String,
    /// A message safe to show to the user.
    pub message: String,
    /// Whether retrying the same request later may succeed.
    pub retryable: bool,
    /// The request ID, as in the `X-Request-Id` header.
    pub request_id: String,
}

impl ApiError {
    /// Builds the body for `error` on request `request_id`. Internal errors
    /// never leak their message.
    #[must_use]
    pub fn from_error(error: &Error, request_id: RequestId) -> Self {
        let message = if error.kind() == ErrorKind::Internal {
            "something went wrong; quote the request ID when reporting it"
                .to_owned()
        } else {
            error.message().to_owned()
        };
        Self {
            code: error.kind().code().to_owned(),
            message,
            retryable: error.kind().is_retryable(),
            request_id: request_id.to_string(),
        }
    }

    /// The HTTP status for an error kind.
    #[must_use]
    pub const fn status(kind: ErrorKind) -> u16 {
        match kind {
            ErrorKind::InvalidInput => 400,
            ErrorKind::Unauthenticated => 401,
            ErrorKind::PermissionDenied => 403,
            ErrorKind::NotFound => 404,
            ErrorKind::Conflict => 409,
            ErrorKind::RateLimited => 429,
            ErrorKind::Unavailable => 503,
            ErrorKind::Timeout => 504,
            _ => 500,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_errors_are_redacted() {
        let id = RequestId::generate();
        let body = ApiError::from_error(
            &Error::new(ErrorKind::Internal, "db password wrong"),
            id,
        );
        assert!(!body.message.contains("password"));
        assert_eq!(body.code, "internal");
        assert_eq!(body.request_id, id.to_string());
        let body =
            ApiError::from_error(&Error::new(ErrorKind::Timeout, "slow"), id);
        assert!(body.retryable);
        assert_eq!(body.message, "slow");
    }

    #[test]
    fn statuses() {
        assert_eq!(ApiError::status(ErrorKind::NotFound), 404);
        assert_eq!(ApiError::status(ErrorKind::Unauthenticated), 401);
        assert_eq!(ApiError::status(ErrorKind::Internal), 500);
    }
}
