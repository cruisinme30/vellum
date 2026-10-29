// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The load tests drive routes the server serves (#441).

use std::path::PathBuf;

use crate::services;

/// Each `/api/v1/…` path in `routes.js`, with `${…}` as `{}` and any
/// query string dropped.
fn load_test_paths() -> Vec<String> {
    let file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../deploy/perf/k6/routes.js");
    let text = std::fs::read_to_string(file).unwrap();
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(at) = rest.find("/api/v1/") {
        let tail = &rest[at..];
        let end = tail.find(['\'', '`', '?', '"']).unwrap_or(tail.len());
        out.push(normalise(&tail[..end].replace("${", "{")));
        rest = &tail[end..];
    }
    out
}

/// A path with every `{…}` parameter written as `{}`.
fn normalise(path: &str) -> String {
    path.split('/')
        .map(|s| if s.starts_with('{') { "{}" } else { s })
        .collect::<Vec<_>>()
        .join("/")
}

#[test]
fn every_load_test_route_is_served() {
    let gateway = services::contract_gateway().unwrap();
    let document = gateway.openapi();
    let served: Vec<String> = document["paths"]
        .as_object()
        .unwrap()
        .keys()
        .map(|p| normalise(&format!("/api{p}")))
        .collect();
    let paths = load_test_paths();
    assert!(paths.len() >= 5, "found {paths:?}");
    for path in paths {
        assert!(
            served.contains(&path),
            "deploy/perf/k6/routes.js drives {path}, which the server does not serve"
        );
    }
}
