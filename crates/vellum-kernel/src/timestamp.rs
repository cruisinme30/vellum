// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A UTC instant with millisecond precision, written as RFC 3339.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

/// A UTC instant, truncated to milliseconds to match ID timestamps.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(OffsetDateTime);

/// Why text is not an RFC 3339 timestamp or is out of range.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid timestamp: {0}")]
pub struct TimestampError(String);

impl Timestamp {
    /// The current time.
    #[must_use]
    pub fn now() -> Self {
        Self::truncate(OffsetDateTime::now_utc())
    }

    /// Builds a timestamp from milliseconds since the Unix epoch.
    ///
    /// # Errors
    ///
    /// Returns an error if the value is outside years -9999 to 9999.
    pub fn from_unix_ms(ms: i64) -> Result<Self, TimestampError> {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
            .map(Self)
            .map_err(|e| TimestampError(e.to_string()))
    }

    /// Milliseconds since the Unix epoch.
    #[must_use]
    pub fn unix_ms(self) -> i64 {
        // Years are bounded to ±9999, so milliseconds always fit in i64.
        i64::try_from(self.0.unix_timestamp_nanos() / 1_000_000).unwrap_or(i64::MAX)
    }

    fn truncate(t: OffsetDateTime) -> Self {
        let t = t.to_offset(UtcOffset::UTC);
        // Re-setting the millisecond clears the sub-millisecond digits.
        Self(t.replace_millisecond(t.millisecond()).unwrap_or(t))
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = self.0.format(&Rfc3339).map_err(|_| fmt::Error)?;
        f.write_str(&text)
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Timestamp({self})")
    }
}

impl FromStr for Timestamp {
    type Err = TimestampError;

    /// Parses RFC 3339 with any offset; stores UTC, truncated to milliseconds.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        OffsetDateTime::parse(text, &Rfc3339)
            .map(Self::truncate)
            .map_err(|e| TimestampError(e.to_string()))
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_offsets_into_utc_milliseconds() -> Result<(), TimestampError> {
        let t: Timestamp = "2026-11-24T10:30:00.123456+02:00".parse()?;
        assert_eq!(t.to_string(), "2026-11-24T08:30:00.123Z");
        assert_eq!(Timestamp::from_unix_ms(t.unix_ms())?, t);
        Ok(())
    }

    #[test]
    fn now_is_whole_milliseconds() -> Result<(), TimestampError> {
        let now = Timestamp::now();
        assert_eq!(Timestamp::from_unix_ms(now.unix_ms())?, now);
        Ok(())
    }

    #[test]
    fn rejects_garbage() {
        assert!("2026-13-01T00:00:00Z".parse::<Timestamp>().is_err());
        assert!("tomorrow".parse::<Timestamp>().is_err());
    }
}
