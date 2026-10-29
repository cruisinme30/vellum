// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Admin → Usage counts people from identity (#473, ADR 0046 D194):
//! accounts by state, and who was seen in the last day, week and month.

#![allow(clippy::unwrap_used, reason = "test helpers fail loudly")]

use url::Url;
use vellum_contracts::Scope;
use vellum_identity::{Identity, IdentityConfig, PeopleCounts, Store};
use vellum_kernel::Timestamp;

const PASSWORD: &str = "correct horse battery staple";
const DAY: i64 = 86_400_000;

fn later(days: i64) -> Timestamp {
    Timestamp::from_unix_ms(Timestamp::now().unix_ms() + days * DAY).unwrap()
}

#[test]
fn people_are_counted_by_state_and_recent_use() {
    let url = Url::parse("https://vellum.northstar.dev").unwrap();
    let identity = Identity::new(IdentityConfig::new(url), Store::memory());
    assert_eq!(identity.people(Timestamp::now()), PeopleCounts::default());

    let priya = identity
        .setup("priya@northstar.dev", "Priya", PASSWORD)
        .unwrap();
    let sam = identity
        .create_user("sam@northstar.dev", "Sam", Some(PASSWORD), false)
        .unwrap();
    let lee = identity
        .create_user("lee@northstar.dev", "Lee", Some(PASSWORD), false)
        .unwrap();
    let _ = identity
        .update_user(lee.id, None, None, Some(true))
        .unwrap();
    let _ = identity
        .sign_in("priya@northstar.dev", PASSWORD, None, None)
        .unwrap();
    let _ = identity
        .create_api_token(sam.id, "CI", &[Scope::Read], None)
        .unwrap();

    let now = identity.people(Timestamp::now());
    assert_eq!((now.users, now.enabled, now.admins), (3, 2, 1));
    assert_eq!((now.sessions, now.api_tokens), (1, 1));
    // Priya signed in; Sam's token was never used.
    assert_eq!((now.active_1d, now.active_7d, now.active_30d), (1, 1, 1));

    let in_ten_days = identity.people(later(10));
    assert_eq!(
        (
            in_ten_days.active_1d,
            in_ten_days.active_7d,
            in_ten_days.active_30d
        ),
        (0, 0, 1)
    );
    assert_eq!(identity.people(later(31)).active_30d, 0);
    assert_eq!(priya.email, "priya@northstar.dev");
}
