// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Saving over concurrent external edits merges them or writes a conflict
//! copy; it never loses either side.

use bytes::Bytes;
use vellum_store::{Base, BlobPath, BlobStore, LocalStore, Precondition, Saved, Writer, save};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const V1: &str = "---\nid: 01JBQ0000000000000000000AA\n---\n# Plan\n\nShip in May.\nHire two.\n";

fn rekey(bytes: &[u8]) -> Bytes {
    let text = String::from_utf8_lossy(bytes).replace("0000AA", "0000BB");
    Bytes::from(text)
}

const ALICE: Writer<'static> = Writer {
    user: "alice",
    date: "2026-10-29",
    reidentify: &rekey,
};

async fn setup()
-> Result<(tempfile::TempDir, LocalStore, BlobPath, Base), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let store = LocalStore::open(dir.path()).await?;
    let path = BlobPath::new("drives/users/alice/Plan.md")?;
    let meta = store
        .put(&path, Bytes::from_static(V1.as_bytes()), Precondition::None)
        .await?;
    let base = Base {
        bytes: Bytes::from_static(V1.as_bytes()),
        etag: Some(meta.etag),
    };
    Ok((dir, store, path, base))
}

async fn text(store: &LocalStore, path: &BlobPath) -> Result<String, Box<dyn std::error::Error>> {
    Ok(String::from_utf8(store.get(path).await?.bytes.to_vec())?)
}

#[tokio::test]
async fn unchanged_blob_is_written() -> TestResult {
    let (_dir, store, path, base) = setup().await?;
    let ours = V1.replace("May", "June");
    let saved = save(&store, &path, base, Bytes::from(ours.clone()), &ALICE).await?;
    assert!(matches!(saved, Saved::Written(_)), "{saved:?}");
    assert_eq!(text(&store, &path).await?, ours);
    Ok(())
}

#[tokio::test]
async fn disjoint_external_edit_is_merged() -> TestResult {
    let (_dir, store, path, base) = setup().await?;
    let external = V1.replace("two", "three");
    let _ = store
        .put(&path, Bytes::from(external), Precondition::None)
        .await?;
    let saved = save(
        &store,
        &path,
        base,
        Bytes::from(V1.replace("May", "June")),
        &ALICE,
    )
    .await?;
    assert!(matches!(saved, Saved::Merged(_)), "{saved:?}");
    assert_eq!(
        text(&store, &path).await?,
        V1.replace("May", "June").replace("two", "three")
    );
    Ok(())
}

#[tokio::test]
async fn clashing_edit_keeps_theirs_and_copies_ours() -> TestResult {
    let (_dir, store, path, base) = setup().await?;
    let external = V1.replace("May", "July");
    let _ = store
        .put(&path, Bytes::from(external.clone()), Precondition::None)
        .await?;
    let saved = save(
        &store,
        &path,
        base,
        Bytes::from(V1.replace("May", "June")),
        &ALICE,
    )
    .await?;
    let Saved::Conflicted { kept, copy } = saved else {
        panic!("expected a conflict: {saved:?}")
    };
    assert_eq!(kept.path, path);
    assert_eq!(text(&store, &path).await?, external);
    assert_eq!(
        copy.path.as_str(),
        "drives/users/alice/Plan (conflict 2026-10-29 alice).md"
    );
    let copied = text(&store, &copy.path).await?;
    assert!(
        copied.contains("June") && copied.contains("0000BB"),
        "{copied}"
    );

    // A second clash the same day gets its own copy.
    let base = Base {
        bytes: Bytes::from(external.clone()),
        etag: Some(kept.etag),
    };
    let _ = store
        .put(
            &path,
            Bytes::from(V1.replace("May", "Aug")),
            Precondition::None,
        )
        .await?;
    let again = save(
        &store,
        &path,
        base,
        Bytes::from(V1.replace("May", "Sep")),
        &ALICE,
    )
    .await?;
    let Saved::Conflicted { copy, .. } = again else {
        panic!("expected a conflict: {again:?}")
    };
    assert_eq!(
        copy.path.file_name(),
        "Plan (conflict 2026-10-29 alice 2).md"
    );
    Ok(())
}

#[tokio::test]
async fn edit_outranks_external_delete() -> TestResult {
    let (_dir, store, path, base) = setup().await?;
    store.delete(&path, Precondition::None).await?;
    let ours = V1.replace("May", "June");
    let saved = save(&store, &path, base, Bytes::from(ours.clone()), &ALICE).await?;
    assert!(matches!(saved, Saved::Written(_)), "{saved:?}");
    assert_eq!(text(&store, &path).await?, ours);
    Ok(())
}

#[tokio::test]
async fn concurrent_creates_never_overwrite() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = LocalStore::open(dir.path()).await?;
    let path = BlobPath::new("logo.png")?;
    let _ = store
        .put(
            &path,
            Bytes::from_static(&[0x89, 0xff, 1]),
            Precondition::None,
        )
        .await?;
    let base = Base {
        bytes: Bytes::new(),
        etag: None,
    };
    let saved = save(
        &store,
        &path,
        base,
        Bytes::from_static(&[0x89, 0xff, 2]),
        &ALICE,
    )
    .await?;
    assert!(matches!(saved, Saved::Conflicted { .. }), "{saved:?}");
    assert_eq!(store.get(&path).await?.bytes.as_ref(), &[0x89, 0xff, 1]);
    Ok(())
}
