// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Upload stress against a real clamd (red team round 4, ADR 0047).
//!
//! Ignored by default; the nightly job starts `ClamAV` with the shipped
//! `clamd.conf` and runs:
//!
//! ```text
//! VELLUM_TEST_CLAMD=127.0.0.1:3310 cargo test -p vellum-security \
//!     --test clamd_stress -- --ignored --test-threads 1
//! ```
//!
//! It checks the clamd config fails closed on what it cannot fully scan,
//! and that a flood of mixed uploads through [`check`] never lets an
//! infected file through, never hangs, and is bounded.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::io::{Cursor, Write as _};
use std::sync::Arc;
use std::time::{Duration, Instant};

use vellum_security::upload::{Clamd, Scanner, UploadError, UploadPolicy, check};
use zip::write::SimpleFileOptions;

const EICAR: &[u8] = b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

fn clamd() -> Clamd {
    let address = std::env::var("VELLUM_TEST_CLAMD").expect("set VELLUM_TEST_CLAMD=host:port");
    Clamd::new(address, Duration::from_secs(30))
}

fn zipped(name: &str, data: &[u8], method: zip::CompressionMethod) -> Vec<u8> {
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    out.start_file(
        name,
        SimpleFileOptions::default().compression_method(method),
    )
    .unwrap();
    out.write_all(data).unwrap();
    out.finish().unwrap().into_inner()
}

fn nested(levels: u32) -> Vec<u8> {
    let mut data = zipped("eicar.txt", EICAR, zip::CompressionMethod::Deflated);
    for level in 1..levels {
        data = zipped(
            &format!("l{level}.zip"),
            &data,
            zip::CompressionMethod::Deflated,
        );
    }
    data
}

/// Sets the "encrypted" flag on every entry, as `zip -P` does.
fn encrypted() -> Vec<u8> {
    let mut bytes = zipped("eicar.txt", EICAR, zip::CompressionMethod::Stored);
    for at in 0..bytes.len().saturating_sub(4) {
        match bytes.get(at..at + 4) {
            Some(b"PK\x03\x04") => bytes[at + 6] |= 1,
            Some(b"PK\x01\x02") => bytes[at + 8] |= 1,
            _ => {}
        }
    }
    bytes
}

const fn infected(result: &Result<(), UploadError>) -> bool {
    matches!(result, Err(UploadError::Infected(_)))
}

#[tokio::test]
#[ignore = "needs a real clamd; run by the nightly job"]
async fn clamd_config_fails_closed() {
    let clamd = clamd();
    assert!(infected(&clamd.scan(EICAR).await), "plain EICAR");
    assert!(
        infected(&clamd.scan(&nested(10)).await),
        "EICAR 10 zips deep"
    );
    // Past MaxRecursion, and in an encrypted zip, clamd used to answer OK.
    assert!(
        infected(&clamd.scan(&nested(25)).await),
        "EICAR 25 zips deep"
    );
    assert!(
        infected(&clamd.scan(&encrypted()).await),
        "EICAR in an encrypted zip"
    );
    // The largest upload Vellum accepts is scanned, not refused.
    clamd.scan(&vec![b'a'; 30 * 1024 * 1024]).await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs a real clamd; run by the nightly job"]
async fn a_flood_of_mixed_uploads_never_lets_malware_through() {
    let clamd = Arc::new(clamd());
    let policy = UploadPolicy::default();
    let bomb = zipped(
        "zeros.bin",
        &vec![0; 64 * 1024 * 1024],
        zip::CompressionMethod::Deflated,
    );
    let clean_text = "Northstar plan\n".repeat(400_000).into_bytes();
    // (name, bytes, is it malware?)
    let files: Vec<(&str, Vec<u8>, bool)> = vec![
        ("eicar.txt", EICAR.to_vec(), true),
        ("eicar.zip", nested(3), true),
        ("deep.zip", nested(25), true),
        ("locked.zip", encrypted(), true),
        ("bomb.zip", bomb, true),
        ("plan.md", clean_text, false),
        ("notes.txt", b"# Northstar\n".to_vec(), false),
    ];
    let started = Instant::now();
    let mut runs = Vec::new();
    for round in 0..8 {
        for (name, bytes, bad) in &files {
            let (clamd, name, bytes, bad) =
                (Arc::clone(&clamd), (*name).to_owned(), bytes.clone(), *bad);
            runs.push(tokio::spawn(async move {
                let at = Instant::now();
                let result = check(policy, Some(clamd.as_ref()), name.clone(), bytes).await;
                (round, name, bad, result.map(|_| ()), at.elapsed())
            }));
        }
    }
    let (mut passed, mut refused, mut busy) = (0, 0, 0);
    for run in runs {
        let (round, name, bad, result, took) = run.await.unwrap();
        assert!(
            took < Duration::from_secs(40),
            "{name} (round {round}) took {took:?}"
        );
        match result {
            Ok(()) => {
                assert!(!bad, "{name} (round {round}) was let through");
                passed += 1;
            }
            Err(UploadError::ScannerUnavailable(_)) => busy += 1,
            Err(e) => {
                assert!(bad, "{name} (round {round}) was refused: {e}");
                refused += 1;
            }
        }
    }
    println!(
        "{passed} clean passed, {refused} malicious refused, {busy} refused as busy, in {:?}",
        started.elapsed()
    );
    assert!(passed > 0 && refused > 0);
}
