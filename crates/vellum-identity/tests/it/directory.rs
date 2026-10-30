// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The people directory behind the sharing and access pickers (D272).

#![allow(clippy::unwrap_used, reason = "tests fail loudly")]

use url::Url;
use vellum_identity::{Identity, IdentityConfig, Store};

fn identity() -> Identity {
    let url = Url::parse("https://vellum.northstar.dev").unwrap();
    Identity::new(IdentityConfig::new(url), Store::memory())
}

#[test]
fn finds_active_people_by_name_handle_or_email() {
    let id = identity();
    let priya = id
        .create_user("priya@northstar.dev", "Priya Raman", None, false)
        .unwrap();
    let _ = id
        .create_user("ana@northstar.dev", "Ana Pri", None, false)
        .unwrap();
    let sam = id
        .create_user("sam@northstar.dev", "Sam Lee", None, false)
        .unwrap();
    let _ = id.update_user(sam.id, None, None, Some(true)).unwrap();

    let names = |q: &str| -> Vec<String> {
        id.search_people(q, 8).into_iter().map(|u| u.name).collect()
    };
    // Names starting with the query come first.
    assert_eq!(names("pri"), ["Priya Raman", "Ana Pri"]);
    assert_eq!(
        names("NORTHSTAR.DEV").len(),
        2,
        "disabled people are left out"
    );
    assert!(names("sam").is_empty());
    // One letter finds nobody, so the directory cannot be walked.
    assert!(names("p").is_empty());
    assert!(names("  ").is_empty());
    assert_eq!(id.search_people("northstar", 1).len(), 1);
    assert_eq!(id.search_people(&priya.handle, 8)[0].id, priya.id);
}
