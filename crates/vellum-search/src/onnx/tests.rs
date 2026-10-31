// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Runs the embedder against the tiny model from [`super::testing`].

use super::testing::{DIMS, write_model};
use super::*;

#[tokio::test]
async fn embeds_mean_pooled_unit_vectors_in_batches_of_mixed_length() {
    let dir = tempfile::tempdir().unwrap();
    let embedder = OnnxEmbedder::new(write_model(dir.path(), true));
    assert_eq!(embedder.dimensions(), DIMS);
    let vectors = embedder
        .embed_passages(&[
            "gateway".into(),
            "gateway deploy deploy".into(),
            String::new(),
        ])
        .await
        .unwrap();
    assert_eq!(vectors.len(), 3);
    // `[CLS] gateway [SEP]`: specials are zero, so the unit vector is e0.
    assert!((vectors[0][0] - 1.0).abs() < 1e-5);
    // One gateway, two deploys: direction (1, 2, 0, 0).
    let norm = 5.0_f32.sqrt();
    assert!((vectors[1][0] - 1.0 / norm).abs() < 1e-5);
    assert!((vectors[1][1] - 2.0 / norm).abs() < 1e-5);
    // Only specials pool to zero, which stays zero.
    assert!(vectors[2].iter().all(|x| x.abs() < 1e-6));
    let query = embedder.embed_query("budget").await.unwrap();
    assert!((query[2] - 1.0).abs() < 1e-5);
}

#[tokio::test]
async fn works_without_token_type_ids() {
    let dir = tempfile::tempdir().unwrap();
    let embedder = OnnxEmbedder::new(write_model(dir.path(), false));
    let v = embedder.embed_query("deploy").await.unwrap();
    assert!((v[1] - 1.0).abs() < 1e-5);
}

#[tokio::test]
async fn wrong_dimensions_and_missing_files_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = write_model(dir.path(), true);
    config.dimensions = 8;
    assert!(
        OnnxEmbedder::new(config.clone())
            .embed_query("gateway")
            .await
            .is_err()
    );
    config.model = dir.path().join("missing.onnx");
    assert!(
        OnnxEmbedder::new(config)
            .embed_query("gateway")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_model_folder_describes_itself() {
    let dir = tempfile::tempdir().unwrap();
    let _ = write_model(dir.path(), true);
    let config = OnnxConfig::from_dir(dir.path()).unwrap();
    assert_eq!(config.dimensions, DIMS);
    assert_eq!(config.max_tokens, 16);
    assert_eq!(config.model_id, "tiny@4");
    assert!(config.query_prefix.is_empty());
    let v = OnnxEmbedder::new(config)
        .embed_query("budget")
        .await
        .unwrap();
    assert!((v[2] - 1.0).abs() < 1e-5);

    std::fs::remove_file(dir.path().join("config.json")).unwrap();
    assert!(OnnxConfig::from_dir(dir.path()).is_err());
    assert!(OnnxConfig::from_dir(&dir.path().join("nowhere")).is_err());
}

#[tokio::test]
async fn an_idle_model_is_dropped_and_loads_again_on_use() {
    let dir = tempfile::tempdir().unwrap();
    let embedder = OnnxEmbedder::new(write_model(dir.path(), true));
    assert!(!embedder.is_loaded().await);
    // Never used: nothing to drop.
    assert!(!embedder.unload_if_idle(Duration::ZERO).await);
    let _ = embedder.embed_query("deploy").await.unwrap();
    assert!(embedder.is_loaded().await);
    assert!(!embedder.unload_if_idle(Duration::from_hours(1)).await);
    assert!(embedder.unload_if_idle(Duration::ZERO).await);
    assert!(!embedder.is_loaded().await);
    let v = embedder.embed_query("deploy").await.unwrap();
    assert!((v[1] - 1.0).abs() < 1e-5);
    assert!(embedder.is_loaded().await);
}
