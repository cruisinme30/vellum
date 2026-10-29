// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Upload hygiene: sniffing, limits, re-encoding and the clamd client.

#![allow(clippy::unwrap_used)]

use std::io::Cursor;
use std::time::Duration;

use image::{ImageFormat, RgbImage};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use vellum_security::upload::{
    Clamd, Kind, Scanner, UploadError, UploadPolicy, safe_file_name, sniff,
};

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    RgbImage::new(width, height)
        .write_to(&mut out, ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

#[test]
fn sniffs_by_content() {
    let cases: &[(&[u8], Option<Kind>)] = &[
        (b"\x89PNG\r\n\x1a\nrest", Some(Kind::Png)),
        (&[0xff, 0xd8, 0xff, 0xe0], Some(Kind::Jpeg)),
        (b"GIF89a...", Some(Kind::Gif)),
        (b"RIFF\0\0\0\0WEBPVP8 ", Some(Kind::Webp)),
        (b"RIFF\0\0\0\0WAVEfmt ", None),
        (b"%PDF-1.7", Some(Kind::Pdf)),
        (b"PK\x03\x04", Some(Kind::Zip)),
        (b"# Northstar roadmap\n", Some(Kind::Text)),
        (
            b"<?xml version=\"1.0\"?>\n<svg xmlns=\"x\"/>",
            Some(Kind::Svg),
        ),
        (b"  <svg></svg>", Some(Kind::Svg)),
        (b"MZ\x90\0\x03", None),
        (b"text with\0nul", None),
        (&[0xc3, 0x28], None),
    ];
    for (bytes, want) in cases {
        assert_eq!(sniff(bytes), *want, "{bytes:?}");
    }
}

#[test]
fn svg_and_zip_are_never_inline() {
    assert!(!Kind::Svg.inline());
    assert!(!Kind::Zip.inline());
    assert!(!Kind::Text.inline());
    assert!(Kind::Png.inline());
    assert_eq!(Kind::Svg.mime(), "image/svg+xml");
}

#[test]
fn extension_must_match_content() {
    let policy = UploadPolicy::default();
    let err = policy.clean("invoice.pdf", png(2, 2)).unwrap_err();
    assert!(matches!(err, UploadError::Mismatch(Kind::Png)));
    let err = policy
        .clean("notes.html", b"<p>hi</p>".to_vec())
        .unwrap_err();
    assert!(matches!(err, UploadError::Mismatch(Kind::Text)));
    let err = policy.clean("tool.exe", b"MZ\x90\0".to_vec()).unwrap_err();
    assert!(matches!(err, UploadError::Unknown));
    let ok = policy
        .clean("Roadmap.MD", b"# Northstar\n".to_vec())
        .unwrap();
    assert_eq!(ok.kind, Kind::Text);
}

#[test]
fn enforces_size_limit() {
    let mut policy = UploadPolicy::default();
    policy.max_bytes = 4;
    let err = policy.clean("a.txt", b"hello".to_vec()).unwrap_err();
    assert!(matches!(err, UploadError::TooLarge(4)));
}

#[test]
fn reencoding_drops_trailing_payload() {
    let mut bytes = png(4, 4);
    let clean_len = bytes.len();
    bytes.extend_from_slice(b"<?php system($_GET['c']); ?>");
    let cleaned = UploadPolicy::default().clean("logo.png", bytes).unwrap();
    assert_eq!(cleaned.kind, Kind::Png);
    assert!(cleaned.bytes.len() <= clean_len + 64);
    assert!(!cleaned.bytes.windows(5).any(|w| w == b"<?php"));
    assert_eq!(sniff(&cleaned.bytes), Some(Kind::Png));
}

#[test]
fn rejects_oversized_and_broken_images() {
    let mut policy = UploadPolicy::default();
    policy.max_dimension = 8;
    let err = policy.clean("big.png", png(16, 4)).unwrap_err();
    assert!(matches!(err, UploadError::Image(_)));

    let mut truncated = png(4, 4);
    truncated.truncate(30);
    let err = UploadPolicy::default()
        .clean("cut.png", truncated)
        .unwrap_err();
    assert!(matches!(err, UploadError::Image(_)));
}

#[test]
fn file_names_are_made_safe() {
    assert_eq!(safe_file_name("../../etc/passwd"), "passwd");
    assert_eq!(safe_file_name("C:\\Users\\a\\plan.docx"), "plan.docx");
    assert_eq!(safe_file_name(".htaccess"), "htaccess");
    assert_eq!(safe_file_name("a\u{0}b\nc<>.txt"), "abc.txt");
    assert_eq!(safe_file_name("..."), "upload");
    assert_eq!(safe_file_name(""), "upload");
    assert!(safe_file_name(&"é".repeat(300)).len() <= 200);
}

/// A minimal clamd that flags any stream containing `EICAR`.
async fn fake_clamd() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    drop(tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut command = [0_u8; 10];
            let _ = socket.read_exact(&mut command).await.unwrap();
            assert_eq!(&command, b"zINSTREAM\0");
            let mut body = Vec::new();
            loop {
                let len = socket.read_u32().await.unwrap() as usize;
                if len == 0 {
                    break;
                }
                let mut chunk = vec![0_u8; len];
                let _ = socket.read_exact(&mut chunk).await.unwrap();
                body.extend_from_slice(&chunk);
            }
            let reply: &[u8] = if body.windows(5).any(|w| w == b"EICAR") {
                b"stream: Eicar-Test-Signature FOUND\0"
            } else {
                b"stream: OK\0"
            };
            socket.write_all(reply).await.unwrap();
        }
    }));
    address
}

#[tokio::test]
async fn clamd_clean_and_infected() {
    let clamd = Clamd::new(fake_clamd().await, Duration::from_secs(5));
    clamd.scan(&vec![b'a'; 200_000]).await.unwrap();
    let err = clamd
        .scan(b"X5O!P%@AP[4\\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*")
        .await
        .unwrap_err();
    match err {
        UploadError::Infected(name) => assert_eq!(name, "Eicar-Test-Signature"),
        other => panic!("unexpected {other:?}"),
    }
}

#[tokio::test]
async fn clamd_unreachable_fails_closed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    drop(listener);
    let err = Clamd::new(address, Duration::from_secs(2))
        .scan(b"hello")
        .await
        .unwrap_err();
    assert!(matches!(err, UploadError::ScannerUnavailable(_)));
}
