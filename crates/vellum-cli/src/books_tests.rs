// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Creating books in `vellum serve` (D322, #566): the drive makes the
//! `.book` folder in a folder its creator may edit, and the creator can
//! list, open and edit the book at once.

use http::StatusCode;
use serde_json::json;

use crate::collections_tests::{get, post, server};
use crate::mcp_tests::world;

#[tokio::test(flavor = "multi_thread")]
async fn a_new_book_is_listed_opened_and_edited_by_its_creator() {
    let dir = tempfile::tempdir().unwrap();
    let (services, app) = server(dir.path()).await;
    let w = world(&app).await;

    let made = post(
        &app,
        &w,
        "/api/v1/books",
        &json!({"folder": "drives/shared/eng", "title": "Field guide"}),
    )
    .await;
    assert_eq!(made.status, StatusCode::OK, "{}", made.text);
    let id = made.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(made.json()["path"], "drives/shared/eng/Field guide.book");
    for file in ["book.yaml", "SUMMARY.md"] {
        assert!(
            dir.path()
                .join("drives/shared/eng/Field guide.book")
                .join(file)
                .exists(),
            "{file}"
        );
    }

    let listed = get(&app, &w, "/api/v1/books").await;
    assert_eq!(listed.status, StatusCode::OK, "{}", listed.text);
    assert_eq!(
        listed.json()["books"][0]["id"],
        id.as_str(),
        "{}",
        listed.text
    );

    let opened = get(&app, &w, &format!("/api/v1/books/{id}")).await;
    assert_eq!(opened.status, StatusCode::OK, "{}", opened.text);
    assert_eq!(opened.json()["title"], "Field guide");

    let section = post(
        &app,
        &w,
        &format!("/api/v1/books/{id}/sections"),
        &json!({"title": "Getting started", "place": {"at": "end"}}),
    )
    .await;
    assert_eq!(section.status, StatusCode::OK, "{}", section.text);

    // The drive shows it where it was made.
    let items = get(
        &app,
        &w,
        &format!("/api/v1/drive/spaces/{}/items", w.space_id),
    )
    .await;
    assert!(items.text.contains("Field guide"), "{}", items.text);

    // Not in a folder that does not exist, and not without a title.
    for (folder, title) in [
        ("drives/shared/eng/Nowhere", "Stray"),
        ("drives/shared/eng", " "),
    ] {
        let refused = post(
            &app,
            &w,
            "/api/v1/books",
            &json!({"folder": folder, "title": title}),
        )
        .await;
        assert_eq!(
            refused.status,
            StatusCode::BAD_REQUEST,
            "{folder}: {}",
            refused.text
        );
    }
    services.shutdown().await;
}
