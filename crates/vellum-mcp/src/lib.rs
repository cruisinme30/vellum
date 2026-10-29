// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Vellum MCP server: the AI door into the workspace.
//!
//! It implements Model Context Protocol revision **2026-07-28**, the
//! stateless revision, over two transports:
//!
//! - **Streamable HTTP** at `/mcp` ([`http`]), protected as an `OAuth` 2.1
//!   resource server ([`auth`]);
//! - **stdio** for `vellum mcp` ([`stdio`]), authenticated by an API
//!   token from the environment.
//!
//! Both feed the same core ([`server::McpServer`]), which serves
//! `server/discover`, the [`tools`], [`resources`] and [`prompts`], and
//! `subscriptions/listen`. Writes made by models land as drafts unless the
//! token carries `write:direct`; every call is audited and rate limited
//! per token; content going to a model passes the injection guard
//! ([`safety`]). [`site`] serves `llms.txt`, the Markdown sitemap and
//! `?format=md`; [`setup`] produces client configuration snippets and the
//! "Test connection" check.
//!
//! The crate talks to the rest of Vellum only through the ports in
//! [`ports`] and [`auth::TokenVerifier`]; [`memory`] is an in-memory
//! implementation used by tests and demos. Design and decisions:
//! ADR 0019 (`docs/adr/0019-mcp-server.md`).

pub mod auth;
pub mod clock;
pub mod http;
pub mod memory;
pub mod ports;
pub mod prompts;
pub mod protocol;
pub mod ratelimit;
pub mod resources;
pub mod safety;
pub mod server;
pub mod setup;
pub mod site;
pub mod stdio;
pub mod tools;
