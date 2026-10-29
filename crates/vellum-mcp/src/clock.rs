// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! RFC 3339 timestamps without a date-time dependency.

use std::time::{SystemTime, UNIX_EPOCH};

/// Converts days since 1970-01-01 to a civil date (Howard Hinnant's
/// algorithm).
const fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "day and month are small and positive"
    )]
    (year, month as u32, day as u32)
}

fn seconds(t: SystemTime) -> (i64, u32) {
    t.duration_since(UNIX_EPOCH).map_or((0, 0), |d| {
        (
            i64::try_from(d.as_secs()).unwrap_or(i64::MAX),
            d.subsec_millis(),
        )
    })
}

/// `2026-10-29T08:05:27.123Z`.
#[must_use]
pub fn rfc3339(t: SystemTime) -> String {
    let (secs, millis) = seconds(t);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// `2026-10-29`.
#[must_use]
pub fn date(t: SystemTime) -> String {
    let (secs, _) = seconds(t);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn formats_known_instants() {
        assert_eq!(rfc3339(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        let t = UNIX_EPOCH + Duration::from_millis(1_793_088_327_123);
        assert_eq!(rfc3339(t), "2026-10-27T08:05:27.123Z");
        assert_eq!(
            date(UNIX_EPOCH + Duration::from_hours(264_384)),
            "2000-02-29"
        );
    }
}
