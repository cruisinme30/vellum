// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Red-team regression tests for the local-folder store (ADR 0030, #272).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use bytes::Bytes;
use futures::StreamExt;
use vellum_store::{BlobPath, BlobStore, LocalStore, Precondition};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn paths_that_escape_on_windows_or_into_git_are_refused() {
    for bad in [
        "C:/Windows/win.ini",
        "drives/shared/a.md:hidden",
        "drives/CON",
        "drives/nul.md",
        "a.md.",
        ".git/config",
        ".Git/hooks/post-checkout",
        "drives/shared/.git/config",
        "GIT~1/config",
    ] {
        assert!(BlobPath::new(bad).is_err(), "{bad:?} should be refused");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn listing_through_a_symlinked_folder_stays_inside_the_root() -> TestResult {
    let outside = tempfile::tempdir()?;
    std::fs::create_dir_all(outside.path().join("inner"))?;
    std::fs::write(outside.path().join("inner/secret.md"), "secret")?;
    let root = tempfile::tempdir()?;
    let store = LocalStore::open(root.path()).await?;
    std::os::unix::fs::symlink(outside.path(), root.path().join("link"))?;

    let listed: Vec<_> = store
        .list(Some(&BlobPath::new("link/inner")?))
        .collect()
        .await;
    assert!(
        listed.iter().all(Result::is_err),
        "listed files outside the root: {listed:?}"
    );
    let whole: Vec<_> = store.list(None).collect().await;
    assert!(
        whole
            .iter()
            .flatten()
            .all(|e| !e.path.as_str().contains("secret"))
    );
    assert!(
        store
            .get(&BlobPath::new("link/inner/secret.md")?)
            .await
            .is_err()
    );
    let write = store
        .put(
            &BlobPath::new("link/inner/planted.md")?,
            Bytes::from_static(b"x"),
            Precondition::None,
        )
        .await;
    assert!(write.is_err());
    assert!(!outside.path().join("inner/planted.md").exists());
    Ok(())
}

#[tokio::test]
async fn a_repository_inside_a_drive_is_not_listed() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = LocalStore::open(root.path()).await?;
    std::fs::create_dir_all(root.path().join("drives/shared/Ops/.git/hooks"))?;
    std::fs::write(root.path().join("drives/shared/Ops/.git/config"), "[core]")?;
    std::fs::write(root.path().join("drives/shared/Ops/Plan.md"), "plan")?;
    let listed: Vec<String> = store
        .list(None)
        .filter_map(|e| async move { e.ok().map(|e| e.path.to_string()) })
        .collect()
        .await;
    assert_eq!(listed, ["drives/shared/Ops/Plan.md"]);
    Ok(())
}
