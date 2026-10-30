// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! W3C trace context over WebSocket.
//!
//! Browsers cannot set headers on a WebSocket upgrade, so the editor sends
//! `traceparent` as a query parameter (or a header, from other clients).
//! It may also send a [`crate::protocol::MSG_TRACE`] frame before any later
//! message it wants traced; that context then parents the server spans for
//! the connection's following messages.

use std::fmt;
use std::str::FromStr;

use opentelemetry::trace::{
    SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId, TraceState,
};
use tracing_opentelemetry::OpenTelemetrySpanExt as _;

/// A parsed W3C `traceparent` value (version `00`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceParent {
    trace_id: TraceId,
    span_id: SpanId,
    flags: TraceFlags,
}

/// Why a `traceparent` value was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("malformed traceparent")]
pub struct TraceParentError;

impl TraceParent {
    /// The remote span context this value names.
    #[must_use]
    pub fn span_context(&self) -> SpanContext {
        SpanContext::new(
            self.trace_id,
            self.span_id,
            self.flags,
            true,
            TraceState::default(),
        )
    }

    /// Makes `span` a child of this remote context.
    pub fn parent(&self, span: &tracing::Span) {
        let cx = opentelemetry::Context::new()
            .with_remote_span_context(self.span_context());
        // Fails only when no OpenTelemetry layer is installed, in which
        // case there is no trace to join.
        let _ignored = span.set_parent(cx);
    }
}

impl FromStr for TraceParent {
    type Err = TraceParentError;

    /// Parses `00-<32 hex trace id>-<16 hex span id>-<2 hex flags>`.
    /// All-zero IDs and the reserved version `ff` are rejected.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut parts = value.trim().split('-');
        let (Some(version), Some(trace), Some(span), Some(flags), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return Err(TraceParentError);
        };
        let lower_hex = |s: &str, len: usize| {
            s.len() == len
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        if version != "00"
            || !lower_hex(trace, 32)
            || !lower_hex(span, 16)
            || !lower_hex(flags, 2)
        {
            return Err(TraceParentError);
        }
        let trace_id =
            TraceId::from_hex(trace).map_err(|_| TraceParentError)?;
        let span_id = SpanId::from_hex(span).map_err(|_| TraceParentError)?;
        let flags =
            u8::from_str_radix(flags, 16).map_err(|_| TraceParentError)?;
        if trace_id == TraceId::INVALID || span_id == SpanId::INVALID {
            return Err(TraceParentError);
        }
        Ok(Self {
            trace_id,
            span_id,
            flags: TraceFlags::new(flags & TraceFlags::SAMPLED.to_u8()),
        })
    }
}

impl fmt::Display for TraceParent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "00-{}-{}-{:02x}",
            self.trace_id,
            self.span_id,
            self.flags.to_u8()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str =
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    #[test]
    fn round_trips_the_w3c_example() {
        let parsed: TraceParent =
            SAMPLE.parse().unwrap_or_else(|e| unreachable!("{e}"));
        assert_eq!(parsed.to_string(), SAMPLE);
        assert!(parsed.span_context().is_sampled());
        assert!(parsed.span_context().is_remote());
    }

    #[test]
    fn rejects_malformed_values() {
        for bad in [
            "",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7",
            "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-x",
            "00-4bf92f3577b34da6a3ce929d0e0e473g-00f067aa0ba902b7-01",
        ] {
            assert_eq!(
                bad.parse::<TraceParent>(),
                Err(TraceParentError),
                "{bad}"
            );
        }
    }
}
