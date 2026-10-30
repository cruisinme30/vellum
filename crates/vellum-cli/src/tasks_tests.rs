// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! My Work through the real gateway: lists in the drive, the calendar
//! feed, CalDAV sign-in and e-mail capture.

use axum::Router;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use http::{StatusCode, header};
use serde_json::{Value, json};
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::GatewayBuilder;
use vellum_tasks::raw::RECIPIENT_HEADER;

use crate::identity_tests::{
    ORIGIN, Reply, csrf, finish, json_request, request, send, signed_in_admin,
};
use crate::services::{Services, VERSION};

struct Fixture {
    app: Router,
    dir: std::path::PathBuf,
    cookie: String,
    csrf: String,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_env(&[]).await
    }

    /// A server with these extra environment variables.
    async fn with_env(env: &'static [(&'static str, &'static str)]) -> Self {
        static NEXT: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "vellum-tasks-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = vellum_kernel::Config::default();
        config.data_dir = dir.clone();
        let services = Services::open(ORIGIN, Some(&dir), None)
            .unwrap()
            .with_workspace(&config, |k| {
                // These tests read the task files on disk.
                if k == crate::workspace::PLAINTEXT_VAR
                    || k == crate::at_rest::PLAINTEXT_VAR
                {
                    return Some("1".to_owned());
                }
                env.iter()
                    .find(|(name, _)| *name == k)
                    .map(|(_, v)| (*v).to_owned())
            })
            .await
            .unwrap();
        assert!(services.all().iter().any(|s| s.name() == "tasks"));
        let mut gateway = GatewayConfig::new(ORIGIN, vec![3; 32]);
        gateway.rate_per_second = 10_000;
        gateway.rate_burst = 10_000;
        let app = services
            .mount(GatewayBuilder::new(gateway, VERSION))
            .build()
            .unwrap()
            .router();
        let cookie = signed_in_admin(&app).await;
        let csrf = csrf(&app, &cookie).await;
        Self {
            app,
            dir,
            cookie,
            csrf,
        }
    }

    async fn post(&self, path: &str, body: Value) -> Reply {
        send(
            &self.app,
            finish(
                json_request("POST", path, &body)
                    .header(header::COOKIE, &self.cookie)
                    .header("x-csrf-token", &self.csrf),
            ),
        )
        .await
    }

    async fn get(&self, path: &str) -> Reply {
        send(
            &self.app,
            finish(request("GET", path).header(header::COOKIE, &self.cookie)),
        )
        .await
    }

    /// The text of the one Markdown file under the data folder whose
    /// name is `name`.
    fn file_named(&self, name: &str) -> Option<String> {
        fn walk(
            dir: &std::path::Path,
            name: &str,
        ) -> Option<std::path::PathBuf> {
            for entry in std::fs::read_dir(dir).ok()?.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(found) = walk(&path, name) {
                        return Some(found);
                    }
                } else if path.file_name().is_some_and(|n| n == name) {
                    return Some(path);
                }
            }
            None
        }
        walk(&self.dir, name).and_then(|p| std::fs::read_to_string(p).ok())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn sign_in(app: &Router, email: &str) -> String {
    let reply = send(
        app,
        finish(json_request(
            "POST",
            "/auth/sign-in",
            &json!({"email": email, "password": "correct horse battery"}),
        )),
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{}", reply.text);
    let set = reply.headers[header::SET_COOKIE].to_str().unwrap();
    set.split(';').next().unwrap().to_owned()
}

fn section_titles(view: &Value) -> Vec<String> {
    view["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|s| s["tasks"].as_array().into_iter().flatten())
        .filter_map(|t| t["title"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[expect(clippy::too_many_lines, reason = "one end-to-end story")]
async fn my_work_runs_on_the_drive() {
    let f = Fixture::new().await;

    // Quick-add creates the Inbox in the person's My Drive.
    let added = f
        .post("/api/v1/tasks", json!({"text": "Pay rent tomorrow p1"}))
        .await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.text);
    let inbox = f.file_named("Inbox.md").expect("an Inbox file");
    assert!(inbox.contains("- [ ] Pay rent {"), "{inbox}");
    assert!(inbox.contains("owner: \"@priya\""), "{inbox}");
    let view = f.get("/api/v1/my-work").await;
    assert_eq!(view.status, StatusCode::OK, "{}", view.text);
    assert!(section_titles(&view.json()).contains(&"Pay rent".to_owned()));

    // A named list lands next to it.
    let list = f
        .post("/api/v1/task-lists", json!({"title": "Groceries"}))
        .await;
    assert_eq!(list.status, StatusCode::OK, "{}", list.text);
    let milk = f
        .post("/api/v1/tasks", json!({"text": "Milk #Groceries"}))
        .await;
    assert_eq!(milk.status, StatusCode::OK, "{}", milk.text);
    assert!(f.file_named("Groceries.md").unwrap().contains("- [ ] Milk"));
    let lists = f.get("/api/v1/task-lists").await;
    assert!(lists.text.contains("Groceries"), "{}", lists.text);

    // Sharing the list lets another person see the items for them.
    let bo = f
        .post(
            "/api/v1/admin/users",
            json!({
                "email": "bo@northstar.dev",
                "name": "Bo",
                "password": "correct horse battery"
            }),
        )
        .await;
    assert_eq!(bo.status, StatusCode::OK, "{}", bo.text);
    let eggs = f
        .post("/api/v1/tasks", json!({"text": "Eggs +bo #Groceries"}))
        .await;
    assert_eq!(eggs.status, StatusCode::OK, "{}", eggs.text);
    let bo_cookie = sign_in(&f.app, "bo@northstar.dev").await;
    let before = send(
        &f.app,
        finish(
            request("GET", "/api/v1/my-work")
                .header(header::COOKIE, &bo_cookie),
        ),
    )
    .await;
    assert!(!section_titles(&before.json()).contains(&"Eggs @bo".to_owned()));
    let list_id = list.json()["doc"].as_str().unwrap().to_owned();
    let shared = f
        .post(
            &format!("/api/v1/task-lists/{list_id}/members"),
            json!({"member": "bo"}),
        )
        .await;
    assert_eq!(shared.status, StatusCode::OK, "{}", shared.text);
    let after = send(
        &f.app,
        finish(
            request("GET", "/api/v1/my-work")
                .header(header::COOKIE, &bo_cookie),
        ),
    )
    .await;
    let titles = section_titles(&after.json());
    assert!(titles.contains(&"Eggs @bo".to_owned()), "{}", after.text);
    assert!(!titles.contains(&"Milk".to_owned()), "{}", after.text);

    // Completing edits the file in place.
    let id = added.json()["id"].as_str().unwrap().to_owned();
    let done = f
        .post(&format!("/api/v1/tasks/{id}/complete"), json!({}))
        .await;
    assert_eq!(done.status, StatusCode::OK, "{}", done.text);
    assert!(f.file_named("Inbox.md").unwrap().contains("- [x] Pay rent"));

    // The calendar feed works with its secret only, and no cookie.
    let feed = f.post("/api/v1/my-work/feed", json!({})).await;
    assert_eq!(feed.status, StatusCode::OK, "{}", feed.text);
    let url = feed.json()["value"].as_str().unwrap().to_owned();
    let path = url.strip_prefix(ORIGIN).unwrap().to_owned();
    let ics = send(&f.app, finish(request("GET", &path))).await;
    assert_eq!(ics.status, StatusCode::OK, "{}", ics.text);
    assert!(ics.text.contains("BEGIN:VCALENDAR"), "{}", ics.text);
    let wrong =
        send(&f.app, finish(request("GET", "/feeds/tasks/nope.ics"))).await;
    assert_eq!(wrong.status, StatusCode::NOT_FOUND);

    // CalDAV ignores the browser cookie, and takes an API token as the
    // Basic password.
    let cookie_only = send(
        &f.app,
        finish(
            request("PROPFIND", "/dav/calendars/priya/")
                .header(header::COOKIE, &f.cookie)
                .header("Depth", "1"),
        ),
    )
    .await;
    assert_eq!(cookie_only.status, StatusCode::UNAUTHORIZED);
    assert!(cookie_only.headers.contains_key(header::WWW_AUTHENTICATE));
    let token = f
        .post(
            "/api/v1/me/tokens",
            json!({"name": "Phone", "scopes": ["read", "write:direct"]}),
        )
        .await;
    assert_eq!(token.status, StatusCode::OK, "{}", token.text);
    let token = token.json()["token"].as_str().unwrap().to_owned();
    let basic = format!("Basic {}", STANDARD.encode(format!("priya:{token}")));
    let home = send(
        &f.app,
        finish(
            request("PROPFIND", "/dav/calendars/priya/")
                .header(header::AUTHORIZATION, &basic)
                .header("Depth", "1"),
        ),
    )
    .await;
    assert_eq!(home.status.as_u16(), 207, "{}", home.text);
    let bad = format!("Basic {}", STANDARD.encode("priya:vlm_wrong"));
    let refused = send(
        &f.app,
        finish(
            request("PROPFIND", "/dav/calendars/priya/")
                .header(header::AUTHORIZATION, &bad)
                .header("Depth", "1"),
        ),
    )
    .await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);

    // A forwarded e-mail lands in the Inbox.
    let address = f.post("/api/v1/my-work/capture-address", json!({})).await;
    assert_eq!(address.status, StatusCode::OK, "{}", address.text);
    let address = address.json()["value"].as_str().unwrap().to_owned();
    let message = "From: Ann <ann@northstar.dev>\r\nTo: x\r\n\
Subject: Renew the domain\r\nMessage-ID: <m1@northstar.dev>\r\n\
Content-Type: text/plain\r\n\r\nIt expires soon.\r\n";
    let mut capture = finish(
        request("POST", "/capture/email").header(RECIPIENT_HEADER, &address),
    );
    *capture.body_mut() = axum::body::Body::from(message);
    let captured = send(&f.app, capture).await;
    assert_eq!(captured.status, StatusCode::ACCEPTED, "{}", captured.text);
    assert!(
        f.file_named("Inbox.md")
            .unwrap()
            .contains("Renew the domain")
    );

    // The Google callback is mounted, and says so when Google is not set up.
    let callback = send(
        &f.app,
        finish(request("GET", "/connect/google/callback?state=x&code=y")),
    )
    .await;
    assert_eq!(
        callback.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        callback.text
    );
    let status = f.get("/api/v1/my-work/google").await;
    assert_eq!(status.status, StatusCode::OK, "{}", status.text);
    assert_eq!(status.json()["configured"], json!(false));

    // Secrets are stored as hashes, never as the token itself.
    let secrets =
        std::fs::read_to_string(f.dir.join(".vellum/tasks/secrets.json"))
            .unwrap();
    let feed_token = path
        .trim_start_matches("/feeds/tasks/")
        .trim_end_matches(".ics");
    assert!(!secrets.contains(feed_token));
}

fn tasks_of_kind(view: &Value, kind: &str) -> Vec<Value> {
    view["sections"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|s| s["tasks"].as_array().into_iter().flatten())
        .filter(|t| t["kind"] == kind)
        .cloned()
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn mentions_and_settings_reach_my_work() {
    let f = Fixture::new().await;
    for (path, body) in [
        (
            "/api/v1/admin/users",
            json!({
                "email": "bo@northstar.dev",
                "name": "Bo",
                "password": "correct horse battery"
            }),
        ),
        (
            "/api/v1/work/teams",
            json!({"key": "ENG", "name": "Engineering"}),
        ),
        (
            "/api/v1/work/issues",
            json!({"team": "ENG", "title": "Fix login"}),
        ),
        (
            "/api/v1/work/comments",
            json!({"target": "ENG-1", "body": "@bo can you look at this?"}),
        ),
    ] {
        let reply = f.post(path, body).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {}", reply.text);
    }
    let bo = sign_in(&f.app, "bo@northstar.dev").await;
    let bo_csrf = csrf(&f.app, &bo).await;
    let bo_get = |path: &'static str| {
        let app = f.app.clone();
        let bo = bo.clone();
        async move {
            send(
                &app,
                finish(request("GET", path).header(header::COOKIE, &bo)),
            )
            .await
        }
    };
    let bo_send = |method: &'static str, path: String, body: Value| {
        let app = f.app.clone();
        let bo = bo.clone();
        let token = bo_csrf.clone();
        async move {
            send(
                &app,
                finish(
                    json_request(method, &path, &body)
                        .header(header::COOKIE, &bo)
                        .header("x-csrf-token", &token),
                ),
            )
            .await
        }
    };

    // The mention waits in Bo's Inbox, and not in the author's My Work.
    let view = bo_get("/api/v1/my-work").await;
    assert_eq!(view.status, StatusCode::OK, "{}", view.text);
    let mentions = tasks_of_kind(&view.json(), "mention");
    assert_eq!(mentions.len(), 1, "{}", view.text);
    assert_eq!(mentions[0]["issue_key"], "ENG-1");
    assert_eq!(mentions[0]["title"], "@bo can you look at this?");
    let mine = f.get("/api/v1/my-work").await;
    assert!(tasks_of_kind(&mine.json(), "mention").is_empty());

    // Dismissing takes it out.
    let comment = mentions[0]["id"]
        .as_str()
        .unwrap()
        .trim_start_matches("mention.")
        .to_owned();
    let dismissed = bo_send(
        "POST",
        format!("/api/v1/my-work/mentions/{comment}/dismiss"),
        json!({}),
    )
    .await;
    assert_eq!(dismissed.status, StatusCode::OK, "{}", dismissed.text);
    let view = bo_get("/api/v1/my-work").await;
    assert!(
        tasks_of_kind(&view.json(), "mention").is_empty(),
        "{}",
        view.text
    );

    // Settings: the server's zone until Bo picks one; bad zones refused.
    let settings = bo_get("/api/v1/my-work/settings").await;
    assert_eq!(settings.status, StatusCode::OK, "{}", settings.text);
    assert_eq!(settings.json()["time_zone"], Value::Null);
    let saved = bo_send(
        "PUT",
        "/api/v1/my-work/settings".to_owned(),
        json!({
            "time_zone": "America/Los_Angeles",
            "day_starts_at": "08:00",
            "digest_at": null
        }),
    )
    .await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert_eq!(saved.json()["effective_time_zone"], "America/Los_Angeles");
    let bad = bo_send(
        "PUT",
        "/api/v1/my-work/settings".to_owned(),
        json!({"time_zone": "Nowhere/Land"}),
    )
    .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);
    let again = bo_get("/api/v1/my-work/settings").await;
    assert_eq!(again.json()["time_zone"], "America/Los_Angeles");
    assert_eq!(again.json()["digest_at"], Value::Null);
    let people =
        std::fs::read_to_string(f.dir.join(".vellum/tasks/people.json"))
            .unwrap();
    assert!(people.contains("America/Los_Angeles"), "{people}");
}

/// A Slack request signed the way Slack signs it.
fn slack_request(secret: &str, body: &str) -> http::Request<axum::body::Body> {
    use hmac::{Hmac, KeyInit as _, Mac as _};
    let ts = jiff::Timestamp::now().as_second().to_string();
    let mut mac =
        Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("v0:{ts}:{body}").as_bytes());
    let signature = mac.finalize().into_bytes().iter().fold(
        String::from("v0="),
        |mut out, b| {
            use std::fmt::Write as _;
            let _ = write!(out, "{b:02x}");
            out
        },
    );
    let mut request = finish(
        request("POST", "/capture/slack")
            .header("X-Slack-Request-Timestamp", ts)
            .header("X-Slack-Signature", signature)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded"),
    );
    *request.body_mut() = axum::body::Body::from(body.to_owned());
    request
}

#[tokio::test(flavor = "multi_thread")]
async fn people_link_their_slack_account_from_the_shortcut() {
    const SECRET: &str = "slack-signing-secret";
    let f = Fixture::with_env(&[(crate::workspace::SLACK_SECRET_VAR, SECRET)])
        .await;
    let payload = json!({
        "type": "message_action",
        "team": {"id": "T01", "domain": "northstar"},
        "channel": {"id": "C01"},
        "user": {"id": "U01"},
        "message": {"text": "Renew the TLS certificate", "ts": "1700000000.0001\
            00"}
    });
    // No `response_url`, so the test never calls Slack.
    let body: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("payload", &payload.to_string())
        .finish();

    // Not linked yet: Slack gets a link with a signed code, nothing is saved.
    let first = send(&f.app, slack_request(SECRET, &body)).await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.text);
    let text = first.json()["text"].as_str().unwrap().to_owned();
    let code = text
        .split("slack_link=")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap()
        .to_owned();
    assert!(text.contains("/my-work?slack_link="), "{text}");
    assert!(
        f.file_named("Inbox.md")
            .is_none_or(|t| !t.contains("Renew the TLS"))
    );

    // A tampered code is refused; the real one links the signed-in person.
    let forged = code.replacen("U01", "U02", 1);
    let refused = f
        .post("/api/v1/my-work/slack-link", json!({"code": forged}))
        .await;
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{}", refused.text);
    let linked = f
        .post("/api/v1/my-work/slack-link", json!({"code": code}))
        .await;
    assert_eq!(linked.status, StatusCode::OK, "{}", linked.text);
    let links =
        std::fs::read_to_string(f.dir.join(".vellum/tasks/slack-users.json"))
            .unwrap();
    assert!(links.contains("T01/U01"), "{links}");

    // Now the shortcut saves the message to their Inbox.
    let saved = send(&f.app, slack_request(SECRET, &body)).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert!(
        saved.json()["text"]
            .as_str()
            .unwrap()
            .starts_with("Saved to your Vellum inbox")
    );
    assert!(
        f.file_named("Inbox.md")
            .unwrap()
            .contains("Renew the TLS certificate")
    );

    // Unlinking stops it.
    let unlinked = send(
        &f.app,
        finish(
            request("DELETE", "/api/v1/my-work/slack-link")
                .header(header::COOKIE, &f.cookie)
                .header(header::ORIGIN, ORIGIN)
                .header("x-csrf-token", &f.csrf),
        ),
    )
    .await;
    assert_eq!(unlinked.status, StatusCode::OK, "{}", unlinked.text);
    let again = send(&f.app, slack_request(SECRET, &body)).await;
    assert!(
        again.json()["text"]
            .as_str()
            .unwrap()
            .contains("slack_link=")
    );
}
