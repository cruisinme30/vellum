// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use super::*;
use crate::conformance;

fn config(location: Location, key: Option<IndexKey>, model: Option<&str>) -> TantivyConfig {
    TantivyConfig {
        location,
        key,
        model: model.map(str::to_owned),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn in_memory_plaintext_conforms() {
    let (backend, opened) = TantivyBackend::open(&config(Location::Memory, None, None)).unwrap();
    assert_eq!(opened, Opened::NeedsRebuild);
    conformance::run(Arc::new(backend)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn encrypted_on_disk_conforms() {
    let dir = tempfile::tempdir().unwrap();
    let key = IndexKey::from_bytes([3; 32]);
    let location = Location::Folder(dir.path().join("index"));
    let (backend, _) = TantivyBackend::open(&config(location, Some(key), None)).unwrap();
    conformance::run(Arc::new(backend)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn reopening_keeps_data_and_a_model_or_key_change_rebuilds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("index");
    let key = IndexKey::from_bytes([5; 32]);
    let corpus = conformance::Corpus::default();
    {
        let (backend, opened) = TantivyBackend::open(&config(
            Location::Folder(path.clone()),
            Some(key.clone()),
            Some("m1"),
        ))
        .unwrap();
        assert_eq!(opened, Opened::NeedsRebuild);
        let changes: Vec<Change> = corpus
            .documents()
            .into_iter()
            .map(|doc| Change::Upsert(crate::backend::IndexEntry { doc, vectors: None }))
            .collect();
        backend.apply(&changes).await.unwrap();
    }
    let probe = |backend: TantivyBackend| async move {
        backend
            .block(corpus.runbook, conformance::bid(3))
            .await
            .unwrap()
    };
    let (backend, opened) = TantivyBackend::open(&config(
        Location::Folder(path.clone()),
        Some(key.clone()),
        Some("m1"),
    ))
    .unwrap();
    assert_eq!(opened, Opened::Ready);
    assert!(probe(backend).await.is_some());

    // A different model empties the index.
    let (backend, opened) = TantivyBackend::open(&config(
        Location::Folder(path.clone()),
        Some(key),
        Some("m2"),
    ))
    .unwrap();
    assert_eq!(opened, Opened::NeedsRebuild);
    assert!(probe(backend).await.is_none());

    // A different key cannot read the stamp, so it rebuilds rather than failing.
    let (_, opened) = TantivyBackend::open(&config(
        Location::Folder(path),
        Some(IndexKey::from_bytes([6; 32])),
        Some("m2"),
    ))
    .unwrap();
    assert_eq!(opened, Opened::NeedsRebuild);
}

#[tokio::test(flavor = "multi_thread")]
async fn odd_queries_never_error() {
    let (backend, _) = TantivyBackend::open(&config(Location::Memory, None, None)).unwrap();
    let corpus = conformance::Corpus::default();
    let spaces = [corpus.eng];
    let filters = crate::model::Filters::default();
    let scope = Scope {
        spaces: &spaces,
        docs: None,
        filters: &filters,
    };
    for text in [
        "\"unclosed",
        "title:",
        "a AND",
        "((",
        "-",
        "*",
        "日本語",
        "c++",
    ] {
        assert!(backend.lexical(text, scope, 5).await.is_ok(), "{text}");
        assert!(backend.titles(text, scope, 5).await.is_ok(), "{text}");
    }
}

#[test]
fn regex_escape_escapes_non_alphanumerics() {
    assert_eq!(regex_escape("ab1"), "ab1");
    assert_eq!(regex_escape("a.b"), "a\\.b");
}
