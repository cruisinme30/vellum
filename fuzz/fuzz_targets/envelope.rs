// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Encrypted blobs read from storage: a tampered or truncated envelope is
//! an error, never a panic or a successful open.

#![no_main]

use libfuzzer_sys::fuzz_target;
use vellum_crypto::{KekId, LocalKeyring, SecretKey};

fuzz_target!(|sealed: &[u8]| {
    let _ = vellum_crypto::kek_of(sealed);
    let _ = vellum_crypto::plaintext_len(sealed);
    let (Ok(id), Ok(key)) = (KekId::new("fuzz"), SecretKey::generate()) else {
        return;
    };
    let keyring = LocalKeyring::new(id, key);
    let opened = futures::executor::block_on(vellum_crypto::open(&keyring, sealed));
    assert!(opened.is_err(), "opened bytes that were never sealed");
});
