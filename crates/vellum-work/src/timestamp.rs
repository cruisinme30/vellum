// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! RFC 3339 timestamps in UTC, as written in Work files.
//!
//! Used through `#[serde(with = "...")]`. Writing always produces
//! `2026-11-02T10:00:00Z`; reading also accepts an offset and a missing
//! seconds field (`2026-11-02T10:00Z`), which people type by hand.

use serde::{Deserialize, Deserializer, Serializer};
use time::format_description::well_known::Rfc3339;
use time::{OffsetDateTime, UtcOffset};

/// Formats a timestamp as RFC 3339 in UTC.
#[must_use]
pub fn format(t: OffsetDateTime) -> String {
    let utc = t.to_offset(UtcOffset::UTC);
    utc.format(&Rfc3339)
        .unwrap_or_else(|_| utc.unix_timestamp().to_string())
}

/// Parses RFC 3339, also accepting `YYYY-MM-DDTHH:MMZ`.
///
/// # Errors
///
/// Returns the parse error text.
pub fn parse(text: &str) -> Result<OffsetDateTime, String> {
    OffsetDateTime::parse(text, &Rfc3339)
        .or_else(|first| {
            // Insert ":00" seconds into "...THH:MM" followed by a zone.
            let t = text.find('T').ok_or_else(|| first.to_string())?;
            let (head, zone) = text.split_at((t + 6).min(text.len()));
            let padded = format!("{head}:00{zone}");
            OffsetDateTime::parse(&padded, &Rfc3339).map_err(|_| first.to_string())
        })
        .map(|t| t.to_offset(UtcOffset::UTC))
}

/// Serialises a required timestamp.
///
/// # Errors
///
/// Only the serializer's own errors.
pub fn serialize<S: Serializer>(t: &OffsetDateTime, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&format(*t))
}

/// Deserialises a required timestamp.
///
/// # Errors
///
/// Fails on text that is not a timestamp.
pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<OffsetDateTime, D::Error> {
    let text = String::deserialize(d)?;
    parse(&text).map_err(serde::de::Error::custom)
}

/// The same for `Option<OffsetDateTime>`.
pub mod option {
    use super::{Deserialize, Deserializer, OffsetDateTime, Serializer, format, parse};

    /// Serialises an optional timestamp.
    ///
    /// # Errors
    ///
    /// Only the serializer's own errors.
    #[allow(clippy::ref_option, reason = "serde's `with` passes &Option<T>")]
    pub fn serialize<S: Serializer>(t: &Option<OffsetDateTime>, s: S) -> Result<S::Ok, S::Error> {
        match t {
            Some(t) => s.serialize_some(&format(*t)),
            None => s.serialize_none(),
        }
    }

    /// Deserialises an optional timestamp.
    ///
    /// # Errors
    ///
    /// Fails on text that is not a timestamp.
    pub fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<OffsetDateTime>, D::Error> {
        Option::<String>::deserialize(d)?
            .map(|text| parse(&text).map_err(serde::de::Error::custom))
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    #[test]
    fn writes_utc_and_reads_hand_typed_forms() {
        let t = datetime!(2026-11-02 10:00 UTC);
        assert_eq!(format(t), "2026-11-02T10:00:00Z");
        assert_eq!(parse("2026-11-02T10:00:00Z").unwrap(), t);
        assert_eq!(parse("2026-11-02T10:00Z").unwrap(), t);
        assert_eq!(parse("2026-11-02T11:00:00+01:00").unwrap(), t);
        assert!(parse("yesterday").is_err());
    }
}
