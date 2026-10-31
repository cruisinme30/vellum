// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum mcp --private` end to end: an AI client talks MCP over stdio to
//! the private agent, which opens Northstar's board space with this
//! device's key.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use serde_json::{Value, json};
use tokio::io::{
    AsyncBufReadExt as _, AsyncWriteExt as _, BufReader, DuplexStream,
};
use vellum_crypto::recipients::{RecipientSecret, seal_for};
use vellum_mcp::auth::{Principal, ScopeSet};
use vellum_mcp::http::request_body;
use vellum_mcp::memory::MemoryAudit;
use vellum_mcp::ports::{Actor, BlockSelector, Workspace as _, WorkspaceError};
use vellum_mcp::server::{McpServer, ServerConfig};
use vellum_private::{DeviceIdentity, PrivateAgent, PrivateSpace};
use vellum_store::{BlobPath, BlobStore, LocalStore, Precondition};

use crate::private::{PrivateWorkspace, parse_space};

const PLAN_ID: &str = "01JDB0ARDP1AN0000000000000";
const PLAN: &str = "---\nid: 01JDB0ARDP1AN0000000000000\ntitle: Q4 plan\n\
    vellum: 1\n---\n\n# Q4 ^blk-q4000001\n\nRaise the seed round in December.\n\
    \n## Risks ^blk-r1sks001\n\nRunway is nine months at the current burn.\n\n\
    ## Hiring ^blk-h1re0001\n\nTwo engineers in January.\n";

async fn put(store: &LocalStore, path: &str, bytes: Vec<u8>) {
    let _ = store
        .put(
            &BlobPath::new(path).unwrap(),
            Bytes::from(bytes),
            Precondition::None,
        )
        .await
        .unwrap();
}

struct World {
    _dir: tempfile::TempDir,
    workspace: PrivateWorkspace,
}

async fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalStore::open(dir.path()).await.unwrap());
    let me = RecipientSecret::generate().unwrap();
    let root = "drives/shared/board";
    put(
        &store,
        &format!("{root}/.vellum-folder.yaml"),
        b"id: 01JDB0ARD00000000000000000\n".to_vec(),
    )
    .await;
    put(
        &store,
        &format!("{root}/plan.md"),
        seal_for(&[me.public()], PLAN.as_bytes()).unwrap(),
    )
    .await;
    let identity = DeviceIdentity::from_secret(me);
    let space = PrivateSpace::open(
        store.as_ref(),
        &identity,
        "board",
        BlobPath::new(root).unwrap(),
    )
    .await
    .unwrap();
    let store: Arc<dyn BlobStore> = store;
    let agent = PrivateAgent::new(identity, store, vec![space]).unwrap();
    let _ = agent.refresh().await.unwrap();
    World {
        _dir: dir,
        workspace: PrivateWorkspace::new(Arc::new(agent)),
    }
}

#[test]
fn space_arguments_are_slug_equals_path() {
    assert_eq!(
        parse_space("board=/drives/shared/board/").unwrap(),
        ("board".to_owned(), "drives/shared/board".to_owned())
    );
    assert!(parse_space("board").is_err());
    assert!(parse_space("=drives/x").is_err());
    assert!(parse_space("board=").is_err());
}

#[tokio::test]
async fn reads_come_from_the_opened_pages() {
    let w = world().await;
    let ws = &w.workspace;
    let actor = Actor::Anonymous;
    let doc = ws.get_doc(&actor, PLAN_ID).await.unwrap();
    assert_eq!(doc.title, "Q4 plan");
    assert_eq!(doc.space, "board");
    assert!(doc.markdown.contains("Runway is nine months"));
    assert_eq!(
        ws.get_doc_by_path(&actor, "/drives/shared/board/plan.md")
            .await
            .unwrap()
            .id,
        PLAN_ID
    );

    let section = ws
        .get_blocks(
            &actor,
            PLAN_ID,
            BlockSelector {
                ids: Vec::new(),
                from: None,
                to: None,
                section: Some("blk-r1sks001".to_owned()),
            },
        )
        .await
        .unwrap();
    assert_eq!(section.len(), 2, "the heading and its paragraph");
    assert_eq!(section[1].heading_path, vec!["Q4", "Risks"]);

    let outline = ws.get_outline(&actor, PLAN_ID).await.unwrap();
    let titles: Vec<_> = outline.iter().map(|n| n.title.as_str()).collect();
    assert_eq!(titles, vec!["Q4", "Risks", "Hiring"]);

    let tree = ws.space_tree(&actor, "board").await.unwrap();
    assert!(tree.markdown.contains(
        "[Q4 plan](vellum://doc/01JDB0ARDP1AN0000000000000) `plan.md`"
    ));
    assert_eq!(
        ws.list_readable(&actor, None, 10)
            .await
            .unwrap()
            .items
            .len(),
        1
    );

    assert!(matches!(
        ws.get_doc(&actor, "01JDB0ARD00000000000000000").await,
        Err(WorkspaceError::NotFound(_))
    ));
    assert!(matches!(
        ws.get_doc(&actor, "not-an-id").await,
        Err(WorkspaceError::NotFound(_))
    ));
    assert!(matches!(
        ws.space_tree(&actor, "elsewhere").await,
        Err(WorkspaceError::NotFound(_))
    ));
}

#[tokio::test]
async fn nothing_is_written_or_exported() {
    let w = world().await;
    let error = w
        .workspace
        .export(
            &Actor::Anonymous,
            vellum_mcp::ports::ExportRequest {
                target: PLAN_ID.to_owned(),
                format: "md".to_owned(),
                section: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(error, WorkspaceError::Forbidden(_)));
}

struct Client {
    input: DuplexStream,
    output: tokio::io::Lines<BufReader<DuplexStream>>,
}

impl Client {
    async fn call(&mut self, id: i64, tool: &str, arguments: Value) -> Value {
        let msg = request_body(
            id,
            "tools/call",
            json!({ "name": tool, "arguments": arguments }),
        );
        self.input
            .write_all(format!("{msg}\n").as_bytes())
            .await
            .unwrap();
        let line = tokio::time::timeout(
            Duration::from_secs(5),
            self.output.next_line(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

#[tokio::test]
async fn an_ai_client_searches_and_cites_but_cannot_write() {
    let w = world().await;
    let server = Arc::new(McpServer::new(
        Arc::new(w.workspace),
        Arc::new(MemoryAudit::default()),
        ServerConfig::default(),
    ));
    let principal = Principal {
        token_id: "private".into(),
        user_id: "local".into(),
        user_name: "local".into(),
        client_id: "stdio".into(),
        client_name: "Claude Desktop".into(),
        scopes: ScopeSet::parse("read"),
    };
    let (client_in, server_in) = tokio::io::duplex(1 << 16);
    let (server_out, client_out) = tokio::io::duplex(1 << 16);
    drop(tokio::spawn(vellum_mcp::stdio::serve(
        server, principal, server_in, server_out,
    )));
    let mut c = Client {
        input: client_in,
        output: BufReader::new(client_out).lines(),
    };

    let found = c
        .call(1, "search", json!({ "query": "runway", "space": "board" }))
        .await;
    let hit = &found["result"]["structuredContent"]["hits"][0];
    assert_eq!(hit["doc_id"], PLAN_ID, "{found}");
    assert_eq!(hit["heading_path"], json!(["Q4", "Risks"]));
    assert!(
        hit["citation_url"]
            .as_str()
            .unwrap()
            .starts_with("vellum://doc/01JDB0ARDP1AN0000000000000#")
    );
    assert!(hit["snippet"].as_str().unwrap().contains("**Runway**"));

    let refused = c
        .call(
            2,
            "create_page",
            json!({
                "parent": "space:board",
                "title": "Leak",
                "markdown": "x"
            }),
        )
        .await;
    let text = refused.to_string();
    assert!(
        refused["error"].is_object() || refused["result"]["isError"] == true,
        "{text}"
    );
}
