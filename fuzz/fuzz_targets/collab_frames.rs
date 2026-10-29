// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Real-time collaboration frames from untrusted clients: decoding never
//! panics, whatever the bytes.

#![no_main]

use libfuzzer_sys::fuzz_target;
use vellum_collab::protocol;

fuzz_target!(|frame: &[u8]| {
    let _ = protocol::decode(frame);
    let _ = protocol::decode_update(frame);
    let _ = protocol::decode_peers(frame);
});
