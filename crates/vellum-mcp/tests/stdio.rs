// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Contract tests of the stdio transport.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream};
use vellum_mcp::auth::{Principal, ScopeSet};
use vellum_mcp::http::request_body;
use vellum_mcp::memory::{MemoryAudit, MemoryWorkspace};
use vellum_mcp::server::{McpServer, Notifier, ServerConfig};
use vellum_mcp::stdio;

struct Client {
    input: DuplexStream,
    output: tokio::io::Lines<BufReader<DuplexStream>>,
    notifier: Notifier,
    done: tokio::task::JoinHandle<std::io::Result<()>>,
}

fn start() -> Client {
    let server = Arc::new(McpServer::new(
        Arc::new(MemoryWorkspace::northstar()),
        Arc::new(MemoryAudit::default()),
        ServerConfig::default(),
    ));
    let notifier = server.notifier();
    let principal = Principal {
        token_id: "tok_cli".into(),
        user_id: "usr_priya".into(),
        user_name: "Priya".into(),
        client_id: "vellum-cli".into(),
        client_name: "Claude Code".into(),
        scopes: ScopeSet::parse("write:draft work"),
    };
    let (client_in, server_in) = tokio::io::duplex(1 << 16);
    let (server_out, client_out) = tokio::io::duplex(1 << 16);
    let done = tokio::spawn(stdio::serve(server, principal, server_in, server_out));
    Client {
        input: client_in,
        output: BufReader::new(client_out).lines(),
        notifier,
        done,
    }
}

impl Client {
    async fn send(&mut self, msg: &Value) {
        let line = format!("{msg}\n");
        assert!(self.input.write_all(line.as_bytes()).await.is_ok());
    }

    async fn recv(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(2), self.output.next_line()).await;
        let text = line.ok().and_then(Result::ok).flatten().unwrap_or_default();
        assert!(!text.contains('\n'));
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }
}

#[tokio::test]
async fn answers_requests_line_by_line() {
    let mut c = start();
    c.send(&request_body(1, "server/discover", json!({}))).await;
    let r = c.recv().await;
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["supportedVersions"][0], "2026-07-28");
    c.send(&request_body(
        2,
        "tools/call",
        json!({ "name": "search", "arguments": { "query": "renderer" } }),
    ))
    .await;
    let r = c.recv().await;
    assert_eq!(r["id"], 2);
    assert_eq!(
        r["result"]["structuredContent"]["hits"][0]["block_id"],
        "blk-pdfr0001"
    );
}

#[tokio::test]
async fn malformed_lines_get_parse_errors() {
    let mut c = start();
    c.send(&json!("not an object")).await;
    assert_eq!(c.recv().await["error"]["code"], -32600);
    let line = b"{oops\n";
    assert!(c.input.write_all(line).await.is_ok());
    assert_eq!(c.recv().await["error"]["code"], -32700);
}

#[tokio::test]
async fn subscriptions_are_correlated_and_cancellable() {
    let mut c = start();
    c.send(&request_body(
        10,
        "subscriptions/listen",
        json!({ "notifications": {
        "resourcesListChanged": true,
        "resourceSubscriptions": ["vellum://doc/01JDLAUNCH0000000000000000"]
    } }),
    ))
    .await;
    let ack = c.recv().await;
    assert_eq!(ack["method"], "notifications/subscriptions/acknowledged");
    assert_eq!(
        ack["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"],
        10
    );
    c.notifier
        .resource_updated("vellum://doc/01JDLAUNCH0000000000000000");
    let n = c.recv().await;
    assert_eq!(n["method"], "notifications/resources/updated");
    assert_eq!(
        n["params"]["_meta"]["io.modelcontextprotocol/subscriptionId"],
        10
    );
    // Other requests interleave on the same channel.
    c.send(&request_body(11, "prompts/list", json!({}))).await;
    assert_eq!(c.recv().await["id"], 11);
    c.send(&json!({ "jsonrpc": "2.0", "method": "notifications/cancelled", "params": { "requestId": 10, "reason": "done" } })).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    c.notifier.resource_list_changed();
    c.send(&request_body(12, "tools/list", json!({}))).await;
    assert_eq!(
        c.recv().await["id"],
        12,
        "nothing more arrives for a cancelled subscription"
    );
}

#[tokio::test]
async fn legacy_handshake_then_plain_requests() {
    let mut c = start();
    c.send(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-11-25", "capabilities": {} } })).await;
    assert_eq!(c.recv().await["result"]["protocolVersion"], "2025-11-25");
    c.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .await;
    c.send(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .await;
    assert_eq!(
        c.recv().await["result"]["tools"].as_array().map(Vec::len),
        Some(15)
    );
}

#[tokio::test]
async fn exits_when_input_closes() {
    let c = start();
    drop(c.input);
    let finished = tokio::time::timeout(Duration::from_secs(2), c.done).await;
    assert!(matches!(finished, Ok(Ok(Ok(())))));
}

#[tokio::test]
async fn answers_every_request_received_before_input_closes() {
    let mut c = start();
    // A subscription stays open until input closes; it must not hold
    // up shutdown.
    c.send(&request_body(
        0,
        "subscriptions/listen",
        json!({ "notifications": { "resourcesListChanged": true } }),
    ))
    .await;
    // Stay under the default burst of 30 calls per token.
    for id in 1..=20 {
        c.send(&request_body(id, "tools/list", json!({}))).await;
    }
    drop(c.input);
    let mut answered = std::collections::BTreeSet::new();
    while let Ok(Ok(Some(line))) =
        tokio::time::timeout(Duration::from_secs(2), c.output.next_line()).await
    {
        let msg: Value = serde_json::from_str(&line).unwrap_or(Value::Null);
        if msg.get("result").is_some() {
            let _ = answered.insert(msg["id"].as_i64().unwrap_or(-1));
        }
    }
    assert_eq!(answered, (1..=20).collect());
    let finished = tokio::time::timeout(Duration::from_secs(2), c.done).await;
    assert!(matches!(finished, Ok(Ok(Ok(())))));
}
