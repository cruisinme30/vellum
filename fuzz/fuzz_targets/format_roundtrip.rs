// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Markdown parser: never panics, never overflows a 1 MiB stack, and its
//! canonical output is a fixed point (parse, serialize, parse again).

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|src: &str| {
    let src = src.to_owned();
    let worker = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let parsed = vellum_format::parse(&src);
            let text = vellum_format::serialize(&parsed.document);
            let again = vellum_format::parse(&text);
            assert_eq!(again.document, parsed.document, "round trip changed the document");
            assert_eq!(vellum_format::serialize(&again.document), text, "not idempotent");
        })
        .expect("spawn");
    if let Err(panic) = worker.join() {
        std::panic::resume_unwind(panic);
    }
});
