// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Collection formula language: parsing and compiling hostile source
//! never panics or overflows the stack.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|src: &str| {
    let src = src.to_owned();
    let worker = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            let _ = vellum_collections::formula::parse(&src);
        })
        .expect("spawn");
    if let Err(panic) = worker.join() {
        std::panic::resume_unwind(panic);
    }
});
