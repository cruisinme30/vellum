// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Google Calendar client against a small fake Google, and the
//! connection files.

use std::sync::{Arc, Mutex as StdMutex};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::TcpListener;
use url::Url;
use vellum_kernel::{ErrorKind, UserId};
use vellum_tasks::google::{Changes, GoogleCalendar as _, GoogleLink, GoogleLinks as _};

use crate::google_http::{Endpoints, FileLinks, GoogleHttp};

/// One request the fake saw.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    target: String,
    auth: Option<String>,
    body: String,
}

type Log = Arc<StdMutex<Vec<Seen>>>;

fn form_value(body: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(body.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

/// What the fake Google answers.
fn answer(seen: &Seen) -> (u16, Value) {
    let path = seen.target.split('?').next().unwrap_or_default();
    let query = seen.target.split_once('?').map_or("", |(_, q)| q);
    match (seen.method.as_str(), path) {
        ("POST", "/token") => {
            match (
                form_value(&seen.body, "grant_type").as_deref(),
                form_value(&seen.body, "refresh_token").as_deref(),
            ) {
                (Some("authorization_code"), _) => (
                    200,
                    json!({"access_token": "a0", "refresh_token": "good", "expires_in": 3600}),
                ),
                (Some("refresh_token"), Some("good")) => {
                    (200, json!({"access_token": "a1", "expires_in": 3600}))
                }
                _ => (400, json!({"error": "invalid_grant"})),
            }
        }
        ("POST", "/revoke") => (200, json!({})),
        _ if !matches!(seen.auth.as_deref(), Some("Bearer a0" | "Bearer a1")) => (401, json!({})),
        ("POST", "/cal/calendars") => (200, json!({"id": "v@group.calendar.google.com"})),
        ("GET", "/cal/calendars/v@group.calendar.google.com/events") => {
            if form_value(query, "syncToken").as_deref() == Some("old") {
                (410, json!({}))
            } else if form_value(query, "pageToken").as_deref() == Some("p2") {
                (200, json!({"items": [{"id": "e2"}], "nextSyncToken": "s1"}))
            } else {
                (200, json!({"items": [{"id": "e1"}], "nextPageToken": "p2"}))
            }
        }
        ("POST", "/cal/calendars/v@group.calendar.google.com/events") => (200, json!({"id": "e3"})),
        ("PATCH", "/cal/calendars/v@group.calendar.google.com/events/e1") => {
            (200, json!({"id": "e1"}))
        }
        ("DELETE", "/cal/calendars/v@group.calendar.google.com/events/e1") => (204, Value::Null),
        _ => (404, json!({})),
    }
}

async fn serve_one(stream: tokio::net::TcpStream, log: Log) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let _ = reader.read_line(&mut line).await?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or_default().to_owned();
    let mut length = 0_usize;
    let mut auth = None;
    loop {
        let mut header = String::new();
        let _ = reader.read_line(&mut header).await?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let value = value.trim();
            if name.eq_ignore_ascii_case("content-length") {
                length = value.parse().unwrap_or(0);
            } else if name.eq_ignore_ascii_case("authorization") {
                auth = Some(value.to_owned());
            }
        }
    }
    let mut body = vec![0_u8; length];
    let _ = reader.read_exact(&mut body).await?;
    let seen = Seen {
        method,
        target,
        auth,
        body: String::from_utf8_lossy(&body).into_owned(),
    };
    let (status, value) = answer(&seen);
    log.lock().expect("log").push(seen);
    let payload = if value.is_null() {
        String::new()
    } else {
        value.to_string()
    };
    let reply = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let mut stream = reader.into_inner();
    stream.write_all(reply.as_bytes()).await?;
    stream.shutdown().await
}

async fn fake_google() -> (Endpoints, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    let log: Log = Arc::default();
    let served = Arc::clone(&log);
    drop(tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(tokio::spawn(serve_one(stream, Arc::clone(&served))));
        }
    }));
    let url = |p: &str| Url::parse(&format!("{base}{p}")).expect("url");
    (
        Endpoints {
            token: url("/token"),
            revoke: url("/revoke"),
            api: url("/cal/"),
        },
        log,
    )
}

fn client(endpoints: Endpoints) -> GoogleHttp {
    GoogleHttp::new(endpoints, "client".into(), "secret".into()).expect("client")
}

#[tokio::test]
async fn the_client_speaks_the_calendar_api() {
    let (endpoints, log) = fake_google().await;
    let google = client(endpoints.clone());

    let grant = google
        .exchange("code".into(), "verifier".into(), "https://v.test/cb".into())
        .await
        .expect("exchange");
    assert_eq!(grant, "good");
    let calendar = google
        .create_calendar(grant.clone(), "Vellum".into(), "Europe/Berlin".into())
        .await
        .expect("calendar");
    assert_eq!(calendar, "v@group.calendar.google.com");

    // Two pages make one change set; an old token asks for a full sync.
    let changes = google
        .changes(grant.clone(), calendar.clone(), None)
        .await
        .expect("changes");
    assert_eq!(
        changes,
        Changes::Page {
            events: vec![json!({"id": "e1"}), json!({"id": "e2"})],
            next_sync_token: Some("s1".into()),
        }
    );
    let expired = google
        .changes(grant.clone(), calendar.clone(), Some("old".into()))
        .await
        .expect("expired");
    assert_eq!(expired, Changes::Expired);

    let id = google
        .insert(grant.clone(), calendar.clone(), json!({"summary": "Write"}))
        .await
        .expect("insert");
    assert_eq!(id, "e3");
    google
        .patch(grant.clone(), calendar.clone(), "e1".into(), json!({}))
        .await
        .expect("patch");
    let missing = google
        .patch(grant.clone(), calendar.clone(), "gone".into(), json!({}))
        .await
        .expect_err("missing");
    assert_eq!(missing.kind(), ErrorKind::NotFound);
    google
        .delete(grant.clone(), calendar.clone(), "e1".into())
        .await
        .expect("delete");
    google
        .delete(grant.clone(), calendar.clone(), "gone".into())
        .await
        .expect("deleting a missing event is fine");
    google.revoke(grant.clone()).await.expect("revoke");

    {
        let seen = log.lock().expect("log").clone();
        // The access token from the exchange was reused, not refreshed.
        let token_calls = seen.iter().filter(|s| s.target == "/token").count();
        assert_eq!(token_calls, 1, "{seen:?}");
        let full = seen.iter().find(|s| s.method == "GET").expect("a listing");
        assert!(full.target.contains("showDeleted=true"), "{}", full.target);
        assert!(full.target.contains("singleEvents=true"), "{}", full.target);
        let exchange = seen.first().expect("exchange");
        assert_eq!(
            form_value(&exchange.body, "code_verifier").as_deref(),
            Some("verifier")
        );
    }

    // A fresh client refreshes; a revoked grant is refused as signed out.
    let fresh = client(endpoints.clone());
    let _ = fresh
        .changes("good".into(), calendar.clone(), None)
        .await
        .expect("refreshed");
    let refused = fresh
        .changes("bad".into(), calendar, None)
        .await
        .expect_err("bad grant");
    assert_eq!(refused.kind(), ErrorKind::Unauthenticated);
}

fn link(calendar: &str) -> GoogleLink {
    GoogleLink {
        refresh_token: "secret-grant".into(),
        calendar: calendar.into(),
        state: vellum_tasks::timeblock::SyncState::default(),
    }
}

#[tokio::test]
async fn connections_live_in_private_files() {
    let dir = tempfile::tempdir().expect("dir");
    let links = FileLinks::new(Some(dir.path().join("google")));
    let ana = UserId::from_u128(1);
    let bo = UserId::from_u128(2);

    assert!(links.link(ana).await.expect("read").is_none());
    assert!(links.linked().await.expect("list").is_empty());
    links.set_link(ana, Some(link("c1"))).await.expect("write");
    links.set_link(bo, Some(link("c2"))).await.expect("write");

    let read = links.link(ana).await.expect("read").expect("linked");
    assert_eq!(read.calendar, "c1");
    assert_eq!(read.refresh_token, "secret-grant");
    let mut linked = links.linked().await.expect("list");
    linked.sort();
    assert_eq!(linked, vec![ana, bo]);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let file = dir.path().join("google").join(format!("{ana}.json"));
        let mode = std::fs::metadata(file).expect("file").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    links.set_link(ana, None).await.expect("remove");
    links.set_link(ana, None).await.expect("remove twice");
    assert!(links.link(ana).await.expect("read").is_none());
    assert_eq!(links.linked().await.expect("list"), vec![bo]);

    let memory = FileLinks::new(None);
    memory.set_link(ana, Some(link("m"))).await.expect("write");
    assert_eq!(memory.linked().await.expect("list"), vec![ana]);
    memory.set_link(ana, None).await.expect("remove");
    assert!(memory.link(ana).await.expect("read").is_none());
}
