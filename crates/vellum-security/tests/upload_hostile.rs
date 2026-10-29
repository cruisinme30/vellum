// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Red team round 4: hostile uploads and a flooded scanner.
//!
//! A real clamd answered "OK" for a zip nested 25 deep and for an
//! encrypted zip, each holding the EICAR test file, and spent 20 seconds
//! on a 150 KB zip that inflates to 150 MB. Uploads are now refused before
//! the scanner sees them, and scans in flight are bounded.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::io::{Cursor, Write as _};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use vellum_security::upload::{
    Clamd, Kind, MAX_ARCHIVE_DEPTH, Scanner, UploadError, UploadPolicy, check,
};
use zip::write::SimpleFileOptions;

const EICAR: &[u8] = b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

fn zip_of(name: &str, data: &[u8], method: zip::CompressionMethod) -> Vec<u8> {
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    out.start_file(
        name,
        SimpleFileOptions::default().compression_method(method),
    )
    .unwrap();
    out.write_all(data).unwrap();
    out.finish().unwrap().into_inner()
}

fn deflated(name: &str, data: &[u8]) -> Vec<u8> {
    zip_of(name, data, zip::CompressionMethod::Deflated)
}

/// `levels` zips, each holding the one inside it, around `inner`.
fn nested(levels: u32, inner: &[u8]) -> Vec<u8> {
    let mut data = deflated("eicar.txt", inner);
    for level in 1..levels {
        data = deflated(&format!("level{level}.zip"), &data);
    }
    data
}

#[test]
fn archives_nested_past_the_limit_are_refused() {
    let policy = UploadPolicy::default();
    let ok = policy
        .clean("ok.zip", nested(MAX_ARCHIVE_DEPTH, EICAR))
        .unwrap();
    assert_eq!(ok.kind, Kind::Zip);
    let err = policy.clean("deep.zip", nested(25, EICAR)).unwrap_err();
    assert!(
        matches!(err, UploadError::Archive(ref m) if m.contains("nested")),
        "{err:?}"
    );
}

#[test]
fn zip_bombs_are_refused_before_the_scan() {
    // 64 MiB of zeros packs into about 64 KB: a 1000x expansion.
    let bomb = deflated("zeros.bin", &vec![0_u8; 64 * 1024 * 1024]);
    assert!(bomb.len() < 200 * 1024);
    let started = Instant::now();
    let err = UploadPolicy::default().clean("bomb.zip", bomb).unwrap_err();
    assert!(
        matches!(err, UploadError::Archive(ref m) if m.contains("expands")),
        "{err:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));

    // The total counts nested archives too.
    let mut policy = UploadPolicy::default();
    policy.max_expanded_bytes = 4 * 1024 * 1024;
    let text: Vec<u8> = (0..6 * 1024 * 1024_u32).map(|i| (i % 251) as u8).collect();
    let err = policy
        .clean("big.zip", deflated("inner.zip", &deflated("t.bin", &text)))
        .unwrap_err();
    assert!(
        matches!(err, UploadError::Archive(ref m) if m.contains("past")),
        "{err:?}"
    );
}

/// Marks every entry of a stored zip as encrypted (general purpose bit 0),
/// as `zip -P` does, without needing a cipher.
fn mark_encrypted(mut bytes: Vec<u8>) -> Vec<u8> {
    let mut at = 0;
    while at + 4 <= bytes.len() {
        let flag = match bytes.get(at..at + 4) {
            Some(b"PK\x03\x04") => Some(at + 6),
            Some(b"PK\x01\x02") => Some(at + 8),
            _ => None,
        };
        if let Some(flag) = flag {
            bytes[flag] |= 1;
        }
        at += 1;
    }
    bytes
}

#[test]
fn encrypted_archives_are_refused() {
    let plain = zip_of("eicar.txt", EICAR, zip::CompressionMethod::Stored);
    let _ = UploadPolicy::default()
        .clean("plain.zip", plain.clone())
        .unwrap();
    let err = UploadPolicy::default()
        .clean("locked.zip", mark_encrypted(plain))
        .unwrap_err();
    assert!(matches!(err, UploadError::Archive(_)), "{err:?}");
}

#[test]
fn office_files_and_corrupt_zips() {
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for name in [
        "[Content_Types].xml",
        "word/document.xml",
        "word/styles.xml",
    ] {
        out.start_file(name, SimpleFileOptions::default()).unwrap();
        out.write_all(
            "<w:document>Northstar plan</w:document>"
                .repeat(400)
                .as_bytes(),
        )
        .unwrap();
    }
    let docx = out.finish().unwrap().into_inner();
    let ok = UploadPolicy::default().clean("Plan.docx", docx).unwrap();
    assert_eq!(ok.kind, Kind::Zip);

    let mut cut = deflated("a.txt", &[b'a'; 10_000]);
    cut.truncate(cut.len() / 2);
    let err = UploadPolicy::default().clean("cut.zip", cut).unwrap_err();
    assert!(matches!(err, UploadError::Archive(_)), "{err:?}");
}

/// A clamd that holds every scan for `hold`, counting how many run at once,
/// and flags EICAR.
async fn slow_clamd(hold: Duration, peak: Arc<AtomicUsize>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let now = Arc::new(AtomicUsize::new(0));
    drop(tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (now, peak) = (Arc::clone(&now), Arc::clone(&peak));
            drop(tokio::spawn(async move {
                let running = now.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = peak.fetch_max(running, Ordering::SeqCst);
                let mut command = [0_u8; 10];
                let _ = socket.read_exact(&mut command).await;
                let mut body = Vec::new();
                while let Ok(len) = socket.read_u32().await {
                    if len == 0 {
                        break;
                    }
                    let mut chunk = vec![0_u8; len as usize];
                    let _ = socket.read_exact(&mut chunk).await;
                    body.extend_from_slice(&chunk);
                }
                // A client that gave up is no longer counted.
                tokio::select! {
                    () = tokio::time::sleep(hold) => {}
                    _ = socket.read_u8() => {
                        let _ = now.fetch_sub(1, Ordering::SeqCst);
                        return;
                    }
                }
                let reply: &[u8] = if body.windows(5).any(|w| w == b"EICAR") {
                    b"stream: Eicar-Test-Signature FOUND\0"
                } else {
                    b"stream: OK\0"
                };
                let _ = socket.write_all(reply).await;
                let _ = now.fetch_sub(1, Ordering::SeqCst);
            }));
        }
    }));
    address
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scans_in_flight_are_bounded_and_floods_fail_fast() {
    let peak = Arc::new(AtomicUsize::new(0));
    let address = slow_clamd(Duration::from_millis(300), Arc::clone(&peak)).await;
    let clamd = Clamd::with_in_flight(address, Duration::from_secs(1), 2);
    let started = Instant::now();
    let scans: Vec<_> = (0..24)
        .map(|_| {
            let clamd = clamd.clone();
            tokio::spawn(async move { clamd.scan(&[b'a'; 4096]).await })
        })
        .collect();
    let mut passed = 0;
    let mut busy = 0;
    for scan in scans {
        match scan.await.unwrap() {
            Ok(()) => passed += 1,
            Err(UploadError::ScannerUnavailable(_)) => busy += 1,
            Err(other) => panic!("unexpected {other:?}"),
        }
    }
    // One more than the bound: a scan that timed out may still be closing.
    assert!(
        peak.load(Ordering::SeqCst) <= 3,
        "clamd saw {peak:?} scans at once"
    );
    assert!(passed >= 4, "only {passed} scans passed");
    assert!(busy > 0, "a flood must be refused, not queued");
    // Every refusal came within the budget, not after the whole queue.
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "{:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn check_runs_the_scanner_on_what_is_stored() {
    let peak = Arc::new(AtomicUsize::new(0));
    let clamd = Clamd::new(
        slow_clamd(Duration::ZERO, peak).await,
        Duration::from_secs(5),
    );
    let policy = UploadPolicy::default();
    let err = check(policy, Some(&clamd), "eicar.txt".to_owned(), EICAR.to_vec())
        .await
        .unwrap_err();
    assert!(matches!(err, UploadError::Infected(_)), "{err:?}");
    let ok = check(
        policy,
        Some(&clamd),
        "notes.md".to_owned(),
        b"# Northstar".to_vec(),
    )
    .await
    .unwrap();
    assert_eq!(ok.kind, Kind::Text);
    let err = check(
        policy,
        Some(&clamd),
        "deep.zip".to_owned(),
        nested(25, EICAR),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, UploadError::Archive(_)), "{err:?}");
    let none: Option<&Clamd> = None;
    let mut small = policy;
    small.max_bytes = 4;
    let err = check(small, none, "a.txt".to_owned(), b"hello".to_vec())
        .await
        .unwrap_err();
    assert!(matches!(err, UploadError::TooLarge(4)));
}
