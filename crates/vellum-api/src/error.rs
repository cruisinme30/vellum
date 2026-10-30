// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Turning kernel errors into HTTP responses.

use axum::Json;
use axum::response::{IntoResponse, Response};
use http::{HeaderValue, StatusCode};
use vellum_contracts::{ApiError, REAUTHENTICATION_REQUIRED};
use vellum_kernel::{Error, ErrorKind, RequestId};

/// A kernel error on its way out as an HTTP response.
#[derive(Debug)]
pub struct ErrorResponse {
    error: Error,
    request_id: RequestId,
}

impl ErrorResponse {
    /// Wraps `error` for request `request_id`.
    #[must_use]
    pub const fn new(error: Error, request_id: RequestId) -> Self {
        Self { error, request_id }
    }

    /// Shorthand for an error of `kind` with a caller-safe message.
    #[must_use]
    pub fn of(kind: ErrorKind, message: &'static str, request_id: RequestId) -> Self {
        Self::new(Error::new(kind, message), request_id)
    }
}

/// The HTTP status for an error kind.
#[must_use]
pub fn status_of(kind: ErrorKind) -> StatusCode {
    StatusCode::from_u16(ApiError::status(kind)).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

impl IntoResponse for ErrorResponse {
    fn into_response(self) -> Response {
        let kind = self.error.kind();
        // A save refused for the secrets in it (R-SE-6): the request was
        // well formed, its content is what cannot be accepted.
        let status = if kind == ErrorKind::InvalidInput
            && self
                .error
                .message()
                .starts_with(vellum_security::secrets::SECRETS_FOUND)
        {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            status_of(kind)
        };
        if kind == ErrorKind::Internal {
            tracing::error!(error = %self.error, source = ?std::error::Error::source(&self.error), "internal error");
        } else {
            tracing::debug!(error = %self.error, "request failed");
        }
        let body = ApiError::from_error(&self.error, self.request_id);
        let mut response = (status, Json(body)).into_response();
        let headers = response.headers_mut();
        match kind {
            ErrorKind::Unauthenticated => {
                let challenge = if self.error.message() == REAUTHENTICATION_REQUIRED {
                    "Bearer error=\"insufficient_user_authentication\""
                } else {
                    "Bearer"
                };
                let _ = headers.insert(
                    http::header::WWW_AUTHENTICATE,
                    HeaderValue::from_static(challenge),
                );
            }
            ErrorKind::RateLimited | ErrorKind::Unavailable => {
                let _ = headers.insert(http::header::RETRY_AFTER, HeaderValue::from_static("1"));
            }
            _ => {}
        }
        response
    }
}
