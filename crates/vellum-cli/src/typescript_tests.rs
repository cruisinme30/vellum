// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The contract between the server and its TypeScript client.

use std::path::PathBuf;

use crate::services;

/// The committed client types, relative to this crate.
fn committed_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/src/api/api.gen.ts")
}

/// Fails when the committed client no longer matches the server's
/// operations. `VELLUM_BLESS=1` rewrites it instead.
#[test]
fn typescript_client_matches_the_server() {
    let gateway = services::contract_gateway().unwrap();
    let generated = vellum_api::typescript::client(gateway.openapi());
    let path = committed_path();
    if std::env::var_os("VELLUM_BLESS").is_some() {
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == generated,
        "web/src/api/api.gen.ts no longer matches the server's operations. \
         Regenerate it with `VELLUM_BLESS=1 cargo test -p vellum-cli typescript_client` \
         and commit the result."
    );
}

/// Every operation the client can call has a path whose parameters are
/// required fields of its input, so the client can always fill the path.
#[test]
fn every_path_parameter_is_a_required_input_field() {
    let gateway = services::contract_gateway().unwrap();
    let document = gateway.openapi();
    let schemas = &document["components"]["schemas"];
    let paths = document["paths"].as_object().unwrap();
    for (path, methods) in paths {
        for (method, op) in methods.as_object().unwrap() {
            let id = op["x-vellum-operation"].as_str().unwrap();
            let input = op["x-vellum-input"]["$ref"]
                .as_str()
                .and_then(|r| r.rsplit('/').next())
                .map_or(&op["x-vellum-input"], |name| &schemas[name]);
            let required: Vec<&str> = input["required"]
                .as_array()
                .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            for param in path
                .split('/')
                .filter_map(|s| s.strip_prefix('{')?.strip_suffix('}'))
            {
                assert!(
                    required.contains(&param),
                    "{method} {path} ({id}): `{param}` is not a required input field"
                );
            }
        }
    }
}
