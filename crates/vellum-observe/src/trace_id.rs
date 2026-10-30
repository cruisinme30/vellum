// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! W3C Trace Context identifiers and the `traceparent` header.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Why an identifier or header could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The value has the wrong length or is not lowercase hex.
    #[error("malformed trace context value")]
    Malformed,
    /// The value is all zeros, which W3C Trace Context forbids.
    #[error("trace context value is all zeros")]
    Zero,
}

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0_u8; N];
    // The OS random source does not fail on supported platforms; if it
    // ever does, fall back to a time-derived value so a request never fails
    // because a trace id could not be drawn.
    if getrandom::fill(&mut bytes).is_err() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos());
        for (slot, byte) in
            bytes.iter_mut().zip(nanos.to_le_bytes().iter().cycle())
        {
            *slot = *byte;
        }
    }
    if bytes.iter().all(|b| *b == 0)
        && let Some(last) = bytes.last_mut()
    {
        *last = 1;
    }
    bytes
}

fn parse_hex<const N: usize>(s: &str) -> Result<[u8; N], ParseError> {
    if s.len() != N * 2
        || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(ParseError::Malformed);
    }
    let mut out = [0_u8; N];
    hex::decode_to_slice(s, &mut out).map_err(|_| ParseError::Malformed)?;
    if out.iter().all(|b| *b == 0) {
        return Err(ParseError::Zero);
    }
    Ok(out)
}

/// A 16-byte W3C trace id, shown as 32 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TraceId([u8; 16]);

impl TraceId {
    /// Draws a new random, non-zero trace id.
    #[must_use]
    pub fn random() -> Self {
        Self(random())
    }

    /// The raw bytes.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 16] {
        self.0
    }
}

impl fmt::Display for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for TraceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TraceId({self})")
    }
}

impl FromStr for TraceId {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_hex(s).map(Self)
    }
}

impl Serialize for TraceId {
    fn serialize<S: Serializer>(
        &self,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TraceId {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// An 8-byte W3C span id, shown as 16 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpanId([u8; 8]);

impl SpanId {
    /// Draws a new random, non-zero span id.
    #[must_use]
    pub fn random() -> Self {
        Self(random())
    }

    /// The raw bytes.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; 8] {
        self.0
    }
}

impl fmt::Display for SpanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl fmt::Debug for SpanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SpanId({self})")
    }
}

impl FromStr for SpanId {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_hex(s).map(Self)
    }
}

/// A parsed `traceparent` header (version 00).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TraceParent {
    /// The trace this request belongs to.
    pub trace_id: TraceId,
    /// The caller's span, the parent of ours.
    pub parent_id: SpanId,
    /// Whether the caller sampled the trace.
    pub sampled: bool,
}

impl TraceParent {
    /// Starts a new sampled trace with a random parent span id.
    #[must_use]
    pub fn new_root() -> Self {
        Self {
            trace_id: TraceId::random(),
            parent_id: SpanId::random(),
            sampled: true,
        }
    }
}

impl fmt::Display for TraceParent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let flags = if self.sampled { "01" } else { "00" };
        write!(f, "00-{}-{}-{flags}", self.trace_id, self.parent_id)
    }
}

impl FromStr for TraceParent {
    type Err = ParseError;

    /// Parses `00-<trace-id>-<parent-id>-<flags>`. Future versions are
    /// accepted when their first four fields have the version-00 shape, as
    /// the specification requires; version `ff` is invalid.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        let mut parts = s.split('-');
        let (Some(version), Some(trace), Some(parent), Some(flags)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(ParseError::Malformed);
        };
        let version = parse_byte(version)?;
        if version == 0xff || (version == 0 && parts.next().is_some()) {
            return Err(ParseError::Malformed);
        }
        let flags = parse_byte(flags)?;
        Ok(Self {
            trace_id: trace.parse()?,
            parent_id: parent.parse()?,
            sampled: flags & 1 == 1,
        })
    }
}

fn parse_byte(s: &str) -> Result<u8, ParseError> {
    if s.len() != 2
        || !s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(ParseError::Malformed);
    }
    u8::from_str_radix(s, 16).map_err(|_| ParseError::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_spec_example() {
        let tp: TraceParent =
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
                .parse()
                .unwrap();
        assert_eq!(tp.trace_id.to_string(), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(tp.parent_id.to_string(), "00f067aa0ba902b7");
        assert!(tp.sampled);
        assert_eq!(
            tp.to_string(),
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
        );
    }

    #[test]
    fn rejects_bad_headers() {
        for bad in [
            "",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7",
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra",
            "0-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-zz",
        ] {
            assert!(bad.parse::<TraceParent>().is_err(), "{bad}");
        }
    }

    #[test]
    fn future_versions_keep_known_fields() {
        let tp: TraceParent =
            "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00-more"
                .parse()
                .unwrap();
        assert!(!tp.sampled);
    }

    #[test]
    fn random_ids_round_trip() {
        let root = TraceParent::new_root();
        let back: TraceParent = root.to_string().parse().unwrap();
        assert_eq!(root, back);
        assert_ne!(TraceId::random(), TraceId::random());
        assert_eq!(format!("{:?}", root.trace_id).len(), 41);
        assert!(format!("{:?}", root.parent_id).starts_with("SpanId("));
        assert_eq!(root.parent_id.to_bytes().len(), 8);
    }

    #[test]
    fn trace_id_serde() {
        let id = TraceId::random();
        let json = serde_json::to_string(&id).unwrap();
        let back: TraceId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
        assert!(serde_json::from_str::<TraceId>("\"zz\"").is_err());
        assert_eq!(id.to_bytes().len(), 16);
    }
}
