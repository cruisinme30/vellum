// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Property tests for timestamps.

use proptest::prelude::*;
use vellum_kernel::Timestamp;

proptest! {
    #[test]
    fn rfc3339_text_round_trips(
        ms in -62_135_596_800_000_i64..253_402_300_799_999
    ) {
        let t = Timestamp::from_unix_ms(ms).unwrap();
        prop_assert_eq!(t.unix_ms(), ms);
        prop_assert_eq!(t.to_string().parse::<Timestamp>(), Ok(t));
        let json = serde_json::to_string(&t).unwrap();
        prop_assert_eq!(serde_json::from_str::<Timestamp>(&json).unwrap(), t);
    }

    #[test]
    fn text_order_matches_time_order(
        a in 0_i64..4_102_444_800_000,
        b in 0_i64..4_102_444_800_000
    ) {
        let (
            x,
            y
        ) = (
            Timestamp::from_unix_ms(a).unwrap(),
            Timestamp::from_unix_ms(b).unwrap()
        );
        prop_assert_eq!(x.cmp(&y), a.cmp(&b));
    }
}
