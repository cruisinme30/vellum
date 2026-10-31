// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Imports with the collections engine connected: databases become
//! collections whose rows keep their pages' bodies and IDs.

#![allow(clippy::unwrap_used, reason = "test helpers fail loudly")]

use std::io::{Cursor, Write as _};

use vellum_export::import::{FileKind, Source, import};
use vellum_jobs::export::CollectionsEngine;
use zip::write::SimpleFileOptions;

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes) in files {
        zip.start_file(*path, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn notion_databases_become_collections() {
    let home = "Home 0123456789abcdef0123456789abcdef";
    let tasks = "Tasks 11111111111111111111111111111111";
    let row = "Rotate keys 22222222222222222222222222222222";
    let home_md = format!(
        "# Home\n\nTrack work in [Rotate keys]({}/{}/{}.md).\n",
        home.replace(' ', "%20"),
        tasks.replace(' ', "%20"),
        row.replace(' ', "%20")
    );
    let csv = "Name,Status,Estimate\nRotate keys,Done,3\nAudit logs,Todo,5\n";
    let row_text = "# Rotate keys\n\nStatus: Done\nEstimate: 3\n\n\
        Rotate the signing keys.\n";
    let set = import(
        Source::Notion,
        "notion.zip",
        &zip_of(&[
            (&format!("{home}.md"), home_md.as_bytes()),
            (&format!("{home}/{tasks}.csv"), csv.as_bytes()),
            (&format!("{home}/{tasks}_all.csv"), csv.as_bytes()),
            (&format!("{home}/{tasks}/{row}.md"), row_text.as_bytes()),
        ]),
        &CollectionsEngine,
    )
    .unwrap();
    let all: Vec<&str> = set.files.iter().map(|f| f.path.as_str()).collect();
    assert!(
        all.contains(&"Home/Tasks.collection/schema.yaml"),
        "{all:?}"
    );
    let rows: Vec<_> = set
        .files
        .iter()
        .filter(|f| f.path.starts_with("Home/Tasks.collection/rows/"))
        .collect();
    assert_eq!(rows.len(), 2, "{all:?}");
    assert!(rows.iter().all(|f| f.kind == FileKind::Collection));
    let rotate = rows
        .iter()
        .find(|f| f.text().contains("Rotate the signing keys."))
        .unwrap()
        .text();
    assert!(!rotate.contains("Status: Done\n\nRotate"), "{rotate}");
    let row_id = rotate
        .lines()
        .find_map(|l| l.strip_prefix("id: "))
        .unwrap()
        .to_owned();
    let home = set.file("Home.md").unwrap().text();
    assert!(
        home.contains(&format!("vellum://doc/{row_id}")),
        "{home}\n{rotate}"
    );
}
