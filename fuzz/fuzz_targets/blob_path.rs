// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Blob paths: whatever is accepted stays inside the root on every system.

#![no_main]

use libfuzzer_sys::fuzz_target;
use vellum_store::BlobPath;

fuzz_target!(|raw: &str| {
    let Ok(path) = BlobPath::new(raw) else {
        return;
    };
    for segment in path.segments() {
        assert!(!segment.is_empty() && segment != "." && segment != "..");
        assert!(!segment.contains(['/', '\\', ':', '\0']), "{segment:?}");
        assert!(!segment.ends_with(['.', ' ']), "{segment:?}");
        assert!(!segment.eq_ignore_ascii_case(".git"), "{segment:?}");
    }
    let joined = std::path::Path::new("/root").join(path.as_str());
    assert!(joined.starts_with("/root"));
    assert_eq!(BlobPath::new(path.as_str()).ok(), Some(path));
});
