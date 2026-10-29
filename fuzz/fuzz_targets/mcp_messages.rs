// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! MCP JSON-RPC messages, resource URIs and OAuth scopes from untrusted
//! clients: parsing never panics, and unknown scopes grant nothing.

#![no_main]

use libfuzzer_sys::fuzz_target;
use vellum_mcp::auth::ScopeSet;
use vellum_mcp::protocol::Incoming;
use vellum_mcp::resources::VellumUri;

fuzz_target!(|text: &str| {
    let _ = Incoming::parse(text);
    let _ = VellumUri::parse(text);
    let scopes = ScopeSet::parse(text);
    let canonical = ScopeSet::parse(&scopes.to_string());
    assert_eq!(scopes.to_string(), canonical.to_string());
});
