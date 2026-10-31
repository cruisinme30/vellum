// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Backups end to end: create, inspect, restore, drill, prune.

#![allow(
    clippy::unwrap_used,
    clippy::cast_possible_truncation,
    reason = "test fixtures fail loudly"
)]

use std::path::Path;

use vellum_admin::backup::{self, CHUNK};
use vellum_crypto::{KekId, LocalKeyring, SecretKey};
use vellum_kernel::ErrorKind;

fn keys(byte: u8) -> LocalKeyring {
    LocalKeyring::new(
        KekId::new("backup").unwrap(),
        SecretKey::from_bytes([byte; 32]),
    )
}

fn seed(root: &Path) {
    let files: [(&str, Vec<u8>); 5] = [
        (
            "drives/shared/eng/Roadmap.md",
            b"---\ntitle: Roadmap\nvellum: 1\n---\n\n# Roadmap\n".to_vec(),
        ),
        ("drives/users/ana/Notes.md", b"Hello Northstar".to_vec()),
        ("drives/users/ana/empty.md", Vec::new()),
        (
            "blobs/large.bin",
            (0..CHUNK + 1234).map(|i| (i % 251) as u8).collect(),
        ),
        (".vellum/keys/kek", b"never in a backup".to_vec()),
    ];
    for (path, bytes) in files {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
    std::fs::write(root.join(".vellum/.x.tmp-1"), b"partial").unwrap();
}

#[tokio::test]
async fn backup_restore_and_drill() {
    let data = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    seed(data.path());
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", data.path().join("link"))
        .unwrap();
    let k = keys(1);

    let info = backup::create(&k, data.path(), out.path(), &[])
        .await
        .unwrap();
    assert_eq!(info.files, 4, "keys and temp files are excluded");
    assert!(info.bytes > CHUNK as u64);
    #[cfg(unix)]
    assert_eq!(info.skipped_links, 1);
    let file = out.path().join(&info.name);
    let raw = std::fs::read(&file).unwrap();
    assert!(
        !raw.windows(15).any(|w| w == b"Hello Northstar"),
        "content is sealed"
    );

    let manifest = backup::read_manifest(&k, &file).await.unwrap();
    let large = manifest
        .files
        .iter()
        .find(|f| f.path == "blobs/large.bin")
        .unwrap();
    assert_eq!(large.chunks, 2);
    let empty = manifest
        .files
        .iter()
        .find(|f| f.path.ends_with("empty.md"))
        .unwrap();
    assert_eq!((empty.size, empty.chunks), (0, 1));

    let target = scratch.path().join("restored");
    let report = backup::restore(&k, &file, &target).await.unwrap();
    assert_eq!(report.files, 4);
    for f in &manifest.files {
        assert_eq!(
            std::fs::read(target.join(&f.path)).unwrap(),
            std::fs::read(data.path().join(&f.path)).unwrap()
        );
    }
    assert!(!target.join(".vellum/keys/kek").exists());

    let err = backup::restore(&k, &file, &target).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "target must be empty");

    let wrong = backup::restore(&keys(2), &file, &scratch.path().join("wrong"))
        .await
        .unwrap_err();
    assert_eq!(wrong.kind(), ErrorKind::Conflict);

    let drill = backup::drill(&k, &file, scratch.path()).await;
    assert!(drill.passed, "{drill:?}");
    assert!(drill.within_rto);
    assert_eq!(drill.files, 4);

    let listed = backup::list(out.path()).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(
        backup::list(&out.path().join("missing"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn damaged_backups_fail_the_drill() {
    let data = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    seed(data.path());
    let k = keys(3);
    let info = backup::create(&k, data.path(), out.path(), &[])
        .await
        .unwrap();
    let file = out.path().join(&info.name);
    let mut raw = std::fs::read(&file).unwrap();
    let mid = raw.len() / 2;
    raw[mid] ^= 0xff;
    std::fs::write(&file, &raw).unwrap();
    let drill = backup::drill(&k, &file, scratch.path()).await;
    assert!(!drill.passed);
    assert!(drill.error.is_some());

    raw.truncate(raw.len() - 10);
    std::fs::write(&file, &raw).unwrap();
    assert!(backup::read_manifest(&k, &file).await.is_err());

    std::fs::write(&file, b"NOPE").unwrap();
    assert_eq!(
        backup::read_manifest(&k, &file).await.unwrap_err().kind(),
        ErrorKind::Conflict
    );
}

#[tokio::test]
async fn prune_keeps_the_newest() {
    let out = tempfile::tempdir().unwrap();
    for stamp in ["20261001T000000Z", "20261002T000000Z", "20261003T000000Z"] {
        std::fs::write(out.path().join(format!("vellum-{stamp}.vbk")), b"x")
            .unwrap();
    }
    std::fs::write(out.path().join("other.txt"), b"x").unwrap();
    let removed = backup::prune(out.path(), 2).await.unwrap();
    assert_eq!(removed, vec!["vellum-20261001T000000Z.vbk"]);
    let left: Vec<String> = backup::list(out.path())
        .await
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert_eq!(
        left,
        vec!["vellum-20261003T000000Z.vbk", "vellum-20261002T000000Z.vbk"]
    );
}
