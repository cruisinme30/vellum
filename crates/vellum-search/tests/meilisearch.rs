// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Meilisearch engine against a throwaway container, through the
//! shared conformance suite.

use std::sync::Arc;

use testcontainers_modules::testcontainers::core::IntoContainerPort as _;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, GenericImage, ImageExt as _};
use vellum_search::conformance;
use vellum_search::meili_index::{MeiliBackend, MeiliConfig};

const KEY: &str = "northstar-test-master-key";

/// A Meilisearch server: in a container, or the one named by
/// `VELLUM_TEST_MEILI_URL` (with `VELLUM_TEST_MEILI_KEY`).
async fn start()
-> Result<(Option<ContainerAsync<GenericImage>>, String, String), Box<dyn std::error::Error>> {
    if let Ok(url) = std::env::var("VELLUM_TEST_MEILI_URL") {
        let key = std::env::var("VELLUM_TEST_MEILI_KEY").unwrap_or_default();
        return Ok((None, url, key));
    }
    let container = GenericImage::new("getmeili/meilisearch", "v1.24")
        .with_exposed_port(7700.tcp())
        .with_env_var("MEILI_MASTER_KEY", KEY)
        .with_env_var("MEILI_NO_ANALYTICS", "true")
        .start()
        .await?;
    let port = container.get_host_port_ipv4(7700).await?;
    let url = format!("http://127.0.0.1:{port}");
    wait_healthy(&url).await?;
    Ok((Some(container), url, KEY.to_owned()))
}

/// Polls `/health` until the server answers, for up to a minute. Meilisearch
/// has no stable start-up log line to wait for.
async fn wait_healthy(url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    for _ in 0..120 {
        let ok = client
            .get(format!("{url}/health"))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        if ok {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err("Meilisearch did not become healthy within a minute".into())
}

#[tokio::test]
async fn meilisearch_conforms() {
    let (_container, url, key) = start().await.unwrap();
    let backend = MeiliBackend::connect(MeiliConfig {
        url,
        api_key: Some(key),
        index: format!("conformance-{}", std::process::id()),
        dimensions: Some(64),
    })
    .await
    .unwrap();
    conformance::run(Arc::new(backend)).await.unwrap();
}
