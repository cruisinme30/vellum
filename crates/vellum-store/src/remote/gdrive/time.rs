// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! RFC 3339 timestamps as Drive writes them (`2026-10-29T08:33:48.123Z`),
//! parsed by hand to avoid a date-time dependency for one field.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Parses an RFC 3339 timestamp with an optional fraction and a `Z` or
/// `±HH:MM` offset. Returns `None` for anything else.
pub(super) fn parse_rfc3339(s: &str) -> Option<SystemTime> {
    let num = |from: usize, to: usize| -> Option<i64> {
        let digits = s.get(from..to)?;
        if digits.bytes().all(|b| b.is_ascii_digit()) {
            digits.parse().ok()
        } else {
            None
        }
    };
    let sep = |at: usize, want: &[u8]| -> Option<()> {
        s.as_bytes()
            .get(at)
            .filter(|b| want.contains(b))
            .map(|_| ())
    };
    sep(4, b"-")?;
    sep(7, b"-")?;
    sep(10, b"Tt ")?;
    sep(13, b":")?;
    sep(16, b":")?;
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
    {
        return None;
    }
    // Leap seconds (`:60`) are folded into the next second.
    if second > 60 {
        return None;
    }

    let mut rest = s.get(19..)?;
    let mut nanos: u32 = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let len = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if len == 0 {
            return None;
        }
        let digits = fraction.get(..len)?;
        // Keep nanosecond precision; ignore anything finer.
        let kept = digits.get(..len.min(9))?;
        let scale = 10_u32.pow(u32::try_from(9 - kept.len()).ok()?);
        nanos = kept.parse::<u32>().ok()? * scale;
        rest = fraction.get(len..)?;
    }

    let offset = match rest {
        "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            if rest.len() != 6 || rest.as_bytes().get(3) != Some(&b':') {
                return None;
            }
            let hh: i64 = rest.get(1..3)?.parse().ok()?;
            let mm: i64 = rest.get(4..6)?.parse().ok()?;
            sign * (hh * 3600 + mm * 60)
        }
    };

    let secs = days_from_civil(year, month, day) * 86_400
        + hour * 3600
        + minute * 60
        + second
        - offset;
    let nanos = Duration::from_nanos(u64::from(nanos));
    if secs >= 0 {
        Some(
            UNIX_EPOCH + Duration::from_secs(u64::try_from(secs).ok()?) + nanos,
        )
    } else {
        Some(UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs()) + nanos)
    }
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar
/// (Howard Hinnant's `days_from_civil`).
const fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era =
        year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: &str) -> Option<Duration> {
        parse_rfc3339(s).and_then(|t| t.duration_since(UNIX_EPOCH).ok())
    }

    #[test]
    fn parses_drive_timestamps() {
        assert_eq!(secs("1970-01-01T00:00:00Z"), Some(Duration::ZERO));
        assert_eq!(
            secs("2000-03-01T00:00:00Z").map(|d| d.as_secs()),
            Some(951_868_800)
        );
        assert_eq!(
            secs("2026-10-29T08:33:48.123Z"),
            Some(Duration::from_millis(1_793_262_828_123))
        );
        assert_eq!(
            secs("2000-03-01T01:00:00+01:00"),
            secs("2000-03-01T00:00:00Z")
        );
        assert_eq!(
            secs("2000-02-29T23:30:00.5-00:30"),
            Some(Duration::from_millis(951_868_800_500))
        );
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "",
            "2026-10-29",
            "2026-13-01T00:00:00Z",
            "2026-10-29T08:33:48",
            "2026-10-29T08:33:48.Z",
            "2026/10/29T08:33:48Z",
            "2026-10-29T08:33:48+0100",
        ] {
            assert!(parse_rfc3339(bad).is_none(), "{bad:?}");
        }
    }
}
