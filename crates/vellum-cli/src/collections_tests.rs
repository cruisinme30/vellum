// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Creating collections in `vellum serve` (D305): a CSV file becomes a
//! collection in a shared drive or one of its folders, for whoever may
//! edit there, and nowhere else. The creator sees and opens it at once
//! (D321, #565).

use axum::Router;
use http::StatusCode;
use serde_json::{Value, json};
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::GatewayBuilder;

use crate::identity_tests::{ORIGIN, Reply, finish, request, send};
use crate::mcp_tests::{World, world};
use crate::services::{Services, VERSION};

/// A server over a fresh plaintext workspace.
pub async fn server(dir: &std::path::Path) -> (Services, Router) {
    let mut config = vellum_kernel::Config::default();
    config.data_dir = dir.to_path_buf();
    let plaintext = |k: &str| {
        (k == crate::workspace::PLAINTEXT_VAR).then(|| "1".to_owned())
    };
    let services = Services::open(ORIGIN, Some(dir), None)
        .unwrap()
        .with_workspace(&config, plaintext)
        .await
        .unwrap();
    let mut gateway = GatewayConfig::new(ORIGIN, vec![3; 32]);
    gateway.rate_per_second = 10_000;
    gateway.rate_burst = 10_000;
    let app = services
        .mount(GatewayBuilder::new(gateway, VERSION))
        .build()
        .unwrap()
        .router();
    (services, app)
}

pub async fn post(app: &Router, w: &World, path: &str, body: &Value) -> Reply {
    send(app, w.post(path, body)).await
}

pub async fn get(app: &Router, w: &World, path: &str) -> Reply {
    send(
        app,
        finish(request("GET", path).header(http::header::COOKIE, &w.cookie)),
    )
    .await
}

async fn from_csv(app: &Router, w: &World, folder: &str, title: &str) -> Reply {
    post(
        app,
        w,
        "/api/v1/collections",
        &json!({
            "folder": folder,
            "title": title,
            "csv": "Name,Annual cost\nAcme,1200\nGlobex,800\n",
        }),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_csv_becomes_a_collection_where_its_creator_may_edit() {
    let dir = tempfile::tempdir().unwrap();
    let (services, app) = server(dir.path()).await;
    let w = world(&app).await;
    let space = post(
        &app,
        &w,
        "/api/v1/drive/spaces",
        &json!({"kind": "shared_drive", "slug": "ops", "name": "Operations"}),
    )
    .await;
    assert_eq!(space.status, StatusCode::OK, "{}", space.text);
    let space_id = space.json()["id"].as_str().unwrap().to_owned();

    // At the top of a shared drive (before D305 this was always refused).
    let made = from_csv(&app, &w, "drives/shared/ops", "Vendors").await;
    assert_eq!(made.status, StatusCode::OK, "{}", made.text);
    assert_eq!(made.json()["rows"], 2);
    assert!(
        dir.path()
            .join("drives/shared/ops/Vendors.collection/schema.yaml")
            .exists()
    );

    // In a folder of it.
    let folder = post(
        &app,
        &w,
        &format!("/api/v1/drive/spaces/{space_id}/folders"),
        &json!({"space_id": space_id, "name": "Finance"}),
    )
    .await;
    assert_eq!(folder.status, StatusCode::OK, "{}", folder.text);
    let nested =
        from_csv(&app, &w, "drives/shared/ops/Finance", "Budgets").await;
    assert_eq!(nested.status, StatusCode::OK, "{}", nested.text);

    // Not in a folder that does not exist, nor outside every space.
    for nowhere in ["drives/shared/ops/Nowhere", "drives/shared/missing"] {
        let refused = from_csv(&app, &w, nowhere, "Stray").await;
        assert_eq!(
            refused.status,
            StatusCode::FORBIDDEN,
            "{nowhere}: {}",
            refused.text
        );
    }
    services.shutdown().await;
}

/// #565: a new collection was written but never put in the drive's folder
/// tree, so the permission check (which follows that tree) hid it from
/// its own creator: "This collection does not exist".
#[tokio::test(flavor = "multi_thread")]
async fn a_new_collection_is_listed_and_opened_by_its_creator() {
    let dir = tempfile::tempdir().unwrap();
    let (services, app) = server(dir.path()).await;
    let w = world(&app).await;
    let space_id = w.space_id.clone();
    let folder = post(
        &app,
        &w,
        &format!("/api/v1/drive/spaces/{space_id}/folders"),
        &json!({"space_id": space_id, "name": "Research"}),
    )
    .await;
    assert_eq!(folder.status, StatusCode::OK, "{}", folder.text);

    let mut made = Vec::new();
    for (folder, title) in [
        ("drives/shared/eng", "Customer interviews"),
        ("drives/shared/eng/Research", "Survey"),
    ] {
        let reply = from_csv(&app, &w, folder, title).await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
        let id = reply.json()["collection"]["id"]
            .as_str()
            .unwrap()
            .to_owned();

        let opened = get(&app, &w, &format!("/api/v1/collections/{id}")).await;
        assert_eq!(opened.status, StatusCode::OK, "{title}: {}", opened.text);
        assert_eq!(opened.json()["title"], title);
        made.push(id);
    }

    let listed = get(&app, &w, "/api/v1/collections").await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    let ids: Vec<String> = listed.json()["collections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_owned())
        .collect();
    for id in &made {
        assert!(ids.contains(id), "{id} not in {ids:?}");
    }
    services.shutdown().await;
}
