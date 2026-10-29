// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Three-way merge of external edits: never panics, and trivial merges
//! return the changed side unchanged.

#![no_main]

use libfuzzer_sys::fuzz_target;
use vellum_store::{MergeKind, Merged, merge3};

#[derive(Debug, arbitrary::Arbitrary)]
struct Input<'a> {
    manifest: bool,
    base: &'a str,
    ours: &'a str,
    theirs: &'a str,
}

fuzz_target!(|input: Input<'_>| {
    let kind = if input.manifest { MergeKind::Manifest } else { MergeKind::Text };
    let _ = merge3(kind, input.base, input.ours, input.theirs);
    assert_eq!(
        merge3(kind, input.base, input.base, input.theirs),
        Merged::Clean(input.theirs.to_owned())
    );
    assert_eq!(
        merge3(kind, input.base, input.ours, input.ours),
        Merged::Clean(input.ours.to_owned())
    );
});
