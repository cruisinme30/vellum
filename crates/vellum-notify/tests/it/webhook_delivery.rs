// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Signed webhook delivery through the real transport to a local receiver.

// Test helpers outside #[test] functions; clippy.toml allows these in tests.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::collections::BTreeSet;

use jiff::Timestamp;
use serde_json::json;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use url::Url;
use vellum_notify::delivery::{Outcome, attempt};
use vellum_notify::endpoint::{SafeTransport, policy};
use vellum_notify::signature::{DEFAULT_TOLERANCE, verify};
use vellum_notify::webhook::{Subscription, TeamScope, prepare};
use vellum_notify::{EventAction, EventType, SIGNATURE_HEADER, WebhookEvent, WebhookSecret};

/// Accepts one request, answers with `status_line`, returns the request.
async fn receiver(status_line: &'static str) -> (u16, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let n = socket.read(&mut chunk).await.unwrap();
            buffer.extend_from_slice(&chunk[..n]);
            let text = String::from_utf8_lossy(&buffer).to_string();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if body.len() >= length {
                    break;
                }
            }
            if n == 0 {
                break;
            }
        }
        socket.write_all(status_line.as_bytes()).await.unwrap();
        socket.shutdown().await.unwrap();
        String::from_utf8(buffer).unwrap()
    });
    (port, handle)
}

fn subscription(port: u16) -> Subscription {
    Subscription {
        id: "hook1".to_owned(),
        url: Url::parse(&format!("http://localhost:{port}/vellum")).unwrap(),
        secrets: vec![WebhookSecret::from_bytes([9; 32])],
        event_types: BTreeSet::from([EventType::Issue]),
        teams: TeamScope::AllPublic,
        enabled: true,
    }
}

fn event() -> WebhookEvent {
    WebhookEvent {
        id: "01JD3Z6Q8W1R2T3Y4U5I6O7P8A".to_owned(),
        event_type: EventType::Issue,
        action: EventAction::Create,
        occurred_at: Timestamp::now(),
        team: Some("ENG".to_owned()),
        actor: None,
        data: json!({"identifier": "ENG-1", "title": "Northstar launch checklist"}),
        updated_from: None,
        url: None,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn delivers_a_verifiable_signature() {
    let (port, server) = receiver("HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n").await;
    let transport = SafeTransport::new(policy(&["localhost".to_owned()])).unwrap();
    let sub = subscription(port);
    let request = prepare(&event(), &sub, Timestamp::now()).unwrap();
    assert_eq!(
        attempt(&transport, &request, "webhook", 0).await,
        Outcome::Delivered
    );

    let raw = server.await.unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let signature = head
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{}: ", SIGNATURE_HEADER.to_ascii_lowercase())))
        .unwrap();
    verify(
        signature,
        body.as_bytes(),
        &sub.secrets[0],
        Timestamp::now(),
        DEFAULT_TOLERANCE,
    )
    .unwrap();
    assert!(head.contains("vellum-event: issue"));
}

#[tokio::test(flavor = "current_thread")]
async fn honours_retry_after_and_refuses_private_targets() {
    let (port, server) = receiver(
        "HTTP/1.1 503 Service Unavailable\r\nRetry-After: 30\r\nContent-Length: 0\r\n\r\n",
    )
    .await;
    let transport = SafeTransport::new(policy(&["localhost".to_owned()])).unwrap();
    let request = prepare(&event(), &subscription(port), Timestamp::now()).unwrap();
    assert_eq!(
        attempt(&transport, &request, "webhook", 0).await,
        Outcome::Retry(Some(30))
    );
    let _ = server.await.unwrap();

    // The strict policy never reaches localhost, whatever the URL says.
    let strict = SafeTransport::new(policy(&[])).unwrap();
    assert_eq!(
        attempt(&strict, &request, "webhook", 0).await,
        Outcome::Rejected
    );
}
