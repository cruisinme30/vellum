// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A frame of a few bytes must not make the server reserve memory for
//! millions of entries it does not carry (issue #311). yrs sizes some
//! tables from the first count in a payload before reading any entry, so
//! a 7-byte sync request once cost 224 MB and 170 ms.
//!
//! This file holds one test so the process's peak memory belongs to it.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use vellum_collab::protocol;

/// Peak resident memory of this process, in kibibytes (Linux only).
fn peak_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

fn var(mut n: u64, out: &mut Vec<u8>) {
    loop {
        let byte = u8::try_from(n & 0x7f).unwrap();
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// `count` encoded on its own: a payload that claims `count` entries and
/// carries none.
fn claim(count: u64) -> Vec<u8> {
    let mut out = Vec::new();
    var(count, &mut out);
    out
}

/// A frame carrying `payload` under `tags`, as a length-prefixed buffer.
fn frame(tags: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = tags.to_vec();
    var(u64::try_from(payload.len()).unwrap(), &mut out);
    out.extend_from_slice(payload);
    out
}

#[test]
fn small_frames_cannot_claim_huge_tables() {
    let count = 1 << 27;
    let sync_request = frame(&[0, 0], &claim(count));
    let awareness = frame(&[1], &claim(count));
    // The bomb hidden behind an ordinary first message.
    let mut second = protocol::encode(&[yrs::sync::Message::AwarenessQuery]);
    second.extend(frame(&[1], &claim(count)));

    let before = peak_kib();
    for f in [&sync_request, &awareness, &second] {
        assert!(protocol::decode(f).is_err(), "{f:02x?} decoded");
    }
    assert!(protocol::decode_update(&claim(count)).is_err());
    if let (Some(before), Some(after)) = (before, peak_kib()) {
        let grown_mib = (after - before) / 1024;
        assert!(
            grown_mib < 32,
            "decoding 4 tiny frames grew memory by {grown_mib} MiB"
        );
    }
}

#[test]
fn counts_that_fit_still_decode() {
    use yrs::updates::encoder::Encode as _;
    use yrs::{Doc, ReadTxn as _, Text as _, Transact as _};
    let doc = Doc::new();
    let text = doc.get_or_insert_text("t");
    text.insert(&mut doc.transact_mut(), 0, "hello");
    let (update, sv) = {
        let txn = doc.transact();
        (
            txn.encode_state_as_update_v1(&yrs::StateVector::default()),
            txn.state_vector().encode_v1(),
        )
    };
    assert!(protocol::decode_update(&update).is_ok());
    let f = frame(&[0, 0], &sv);
    assert!(protocol::decode(&f).is_ok());
    // An empty state vector (count 0, nothing after it) is valid.
    assert!(protocol::decode(&frame(&[0, 0], &claim(0))).is_ok());
}
