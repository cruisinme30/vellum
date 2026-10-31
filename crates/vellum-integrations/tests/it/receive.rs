// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The HTTP receivers, end to end over the real issue tracker engine.

// Test helpers outside #[test] functions; clippy.toml allows these in tests.
#![allow(clippy::unwrap_used, clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use hmac::{Hmac, KeyInit, Mac};
use jiff::Timestamp;
use serde_json::{Value, json};
use sha2::Sha256;
use tower::ServiceExt as _;
use vellum_integrations::git::automation::{IssueLinks, Mapping};
use vellum_integrations::git::event::{PrRef, Provider};
use vellum_integrations::git::service::{LinkStore, Repository, Settings};
use vellum_integrations::intake::Routing;
use vellum_integrations::intake::email::{EmailSettings, reply_address};
use vellum_integrations::model::{
    IssueKey, StateCategory, WorkCommands, WorkError,
};
use vellum_integrations::receive::{ReceiverConfig, Receivers};
use vellum_integrations::work::{Commit, CommitSink, GraphWork};
use vellum_kernel::TeamId;
use vellum_work::{Comments, Team, TeamKey, UserRef, WorkGraph};

struct Sink;

impl CommitSink for Sink {
    async fn commit(&self, _commit: Commit) -> Result<(), WorkError> {
        Ok(())
    }
}

#[derive(Default)]
struct Links(Mutex<BTreeMap<IssueKey, IssueLinks>>);

impl LinkStore for Links {
    async fn load(&self, key: &IssueKey) -> Result<IssueLinks, WorkError> {
        Ok(self.0.lock().unwrap().get(key).cloned().unwrap_or_default())
    }

    async fn save(
        &self,
        key: &IssueKey,
        links: &IssueLinks,
    ) -> Result<(), WorkError> {
        let _ = self.0.lock().unwrap().insert(key.clone(), links.clone());
        Ok(())
    }

    async fn issues_for_pr(
        &self,
        pr: &PrRef,
    ) -> Result<BTreeSet<IssueKey>, WorkError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, l)| l.prs.iter().any(|p| &p.pr == pr))
            .map(|(k, _)| k.clone())
            .collect())
    }
}

const GITHUB: &[u8] = b"github-secret";
const SLACK: &[u8] = b"slack-secret";
const SENTRY: &[u8] = b"sentry-secret";
const MAIL: &[u8] = b"mail-token";
const REPLY: &[u8] = b"reply-secret";

fn now() -> Timestamp {
    "2026-10-29T09:00:00Z".parse().unwrap()
}

fn hmac_hex(key: &[u8], parts: &[&[u8]]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).unwrap();
    for part in parts {
        mac.update(part);
    }
    mac.finalize()
        .into_bytes()
        .iter()
        .fold(String::new(), |mut out, b| {
            use std::fmt::Write as _;
            let _ = write!(out, "{b:02x}");
            out
        })
}

fn routing() -> Routing {
    Routing {
        default_team: "ENG".to_owned(),
        teams: std::iter::once("ENG".to_owned()).collect(),
    }
}

fn email_settings(secret: &[u8]) -> EmailSettings {
    EmailSettings {
        routing: routing(),
        intake_domain: "intake.northstar.dev".to_owned(),
        plus_prefix: "issues".to_owned(),
        reply_domain: "northstar.dev".to_owned(),
        reply_secret: secret.to_vec(),
    }
}

fn unconfigured() -> ReceiverConfig {
    ReceiverConfig {
        github_secret: Vec::new(),
        gitlab_token: String::new(),
        slack_signing_secret: Vec::new(),
        sentry_client_secret: Vec::new(),
        mail_token: Vec::new(),
        git: Settings::default(),
        routing: routing(),
        sentry_projects: BTreeMap::new(),
        email: email_settings(b""),
    }
}

async fn setup(configured: bool) -> (axum::Router, GraphWork<Sink>) {
    let mut graph = WorkGraph::new();
    let team = Team::new(
        TeamId::generate(),
        TeamKey::parse("ENG").unwrap(),
        "Engineering",
    )
    .unwrap();
    graph.insert_team(team).unwrap();
    let graph = Arc::new(Mutex::new(graph));
    let comments = Arc::new(Mutex::new(Comments::new()));
    // Two handles on the same engine: one for the receivers, one to check.
    let handle = || {
        GraphWork::new(
            Arc::clone(&graph),
            Arc::clone(&comments),
            Sink,
            UserRef::parse("vellum-bot").unwrap(),
        )
        .with_clock(now)
    };
    let work = handle();
    let mut config = unconfigured();
    if configured {
        let states = work.states("ENG").await.unwrap();
        config.github_secret = GITHUB.to_vec();
        "gitlab-token".clone_into(&mut config.gitlab_token);
        config.slack_signing_secret = SLACK.to_vec();
        config.sentry_client_secret = SENTRY.to_vec();
        config.mail_token = MAIL.to_vec();
        config.email = email_settings(REPLY);
        config.git = Settings {
            repositories: vec![Repository {
                provider: Provider::GitHub,
                repo: "northstar/api".to_owned(),
                teams: std::iter::once("ENG".to_owned()).collect(),
            }],
            mappings: std::iter::once((
                "ENG".to_owned(),
                Mapping::default_for(&states),
            ))
            .collect(),
        };
    }
    let receivers =
        Receivers::new(handle(), Links::default(), config).with_clock(now);
    (Arc::new(receivers).router(), work)
}

async fn send(
    router: &axum::Router,
    path: &str,
    headers: &[(&str, &str)],
    body: Vec<u8>,
) -> (StatusCode, Value) {
    let mut request = Request::post(path);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn key(text: &str) -> IssueKey {
    IssueKey::parse(text).unwrap()
}

#[tokio::test]
async fn every_route_refuses_when_no_secret_is_configured() {
    let (router, _) = setup(false).await;
    for path in ["/github", "/gitlab", "/slack", "/sentry", "/email"] {
        // Even a request "signed" with the empty key is refused.
        let body = b"{}".to_vec();
        let signature = hmac_hex(b"", &[&body]);
        let (status, json) = send(
            &router,
            path,
            &[
                ("X-Hub-Signature-256", &format!("sha256={signature}")),
                ("Sentry-Hook-Signature", &signature),
                ("X-Gitlab-Token", ""),
                ("Authorization", "Bearer "),
            ],
            body,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(json["error"], "integration not configured");
    }
}

#[tokio::test]
async fn github_pull_requests_move_issues() {
    let (router, work) = setup(true).await;
    let issue = work
        .create_issue(vellum_integrations::model::IssueDraft::new(
            "ENG",
            "Fix login",
            vellum_integrations::model::Source::App,
        ))
        .await
        .unwrap();
    let body = serde_json::to_vec(&json!({
        "action": "opened",
        "repository": {"full_name": "northstar/api", "default_branch": "main"},
        "pull_request": {
            "number": 12, "title": "Fix login", "body": format!(
                "Fixes {issue}"
            ),
            "html_url": "https://github.com/northstar/api/pull/12",
            "draft": false, "merged": false, "head": {"ref": "fix-login"}
        }
    }))
    .unwrap();

    let forged = format!("sha256={}", hmac_hex(b"wrong", &[&body]));
    let (status, _) = send(
        &router,
        "/github",
        &[
            ("X-GitHub-Event", "pull_request"),
            ("X-Hub-Signature-256", &forged),
        ],
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_ne!(
        work.issue(&issue).await.unwrap().unwrap().state.id,
        "in-review"
    );

    let signed = format!("sha256={}", hmac_hex(GITHUB, &[&body]));
    let (status, json) = send(
        &router,
        "/github",
        &[
            ("X-GitHub-Event", "pull_request"),
            ("X-Hub-Signature-256", &signed),
        ],
        body,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{json}");
    assert_eq!(json["applied"], 1);
    assert_eq!(
        work.issue(&issue).await.unwrap().unwrap().state.id,
        "in-review"
    );

    let ping = b"{\"zen\":\"Keep it logically awesome.\"}".to_vec();
    let signed = format!("sha256={}", hmac_hex(GITHUB, &[&ping]));
    let (status, _) = send(
        &router,
        "/github",
        &[("X-GitHub-Event", "ping"), ("X-Hub-Signature-256", &signed)],
        ping,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn sentry_alerts_are_filed_once() {
    let (router, work) = setup(true).await;
    let body = serde_json::to_vec(&json!({
        "action": "created",
        "data": {"issue": {
            "id": "4711", "shortId": "WEB-3K", "title": "TypeError in checkout",
            "culprit": "cart.ts", "level": "error",
            "permalink": "https://northstar.sentry.io/issues/4711/",
            "project": {"slug": "web-app"}
        }}
    }))
    .unwrap();
    let signature = hmac_hex(SENTRY, &[&body]);
    let headers = [
        ("Sentry-Hook-Resource", "issue"),
        ("Sentry-Hook-Signature", signature.as_str()),
    ];
    let (status, first) =
        send(&router, "/sentry", &headers, body.clone()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{first}");
    let (_, again) = send(&router, "/sentry", &headers, body).await;
    assert_eq!(first["issue"], again["issue"]);
    let filed = work
        .issue(&key(first["issue"].as_str().unwrap()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(filed.state.category, StateCategory::Triage);
}

#[tokio::test]
async fn slack_commands_answer_the_user() {
    let (router, _) = setup(true).await;
    let body =
        b"command=%2Fvellum&text=Export+button+does+nothing&user_id=U1&r\
        esponse_url=https%3A%2F%2Fhooks.slack.com%2Fx"
            .to_vec();
    let sign = |ts: &str| {
        format!(
            "v0={}",
            hmac_hex(SLACK, &[b"v0:", ts.as_bytes(), b":", &body])
        )
    };
    let fresh = now().as_second().to_string();
    let (status, json) = send(
        &router,
        "/slack",
        &[
            ("X-Slack-Request-Timestamp", &fresh),
            ("X-Slack-Signature", &sign(&fresh)),
        ],
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["text"], "Filed ENG-1 in triage.");

    // The same signed request sent again inside the window files nothing.
    let (status, json) = send(
        &router,
        "/slack",
        &[
            ("X-Slack-Request-Timestamp", &fresh),
            ("X-Slack-Signature", &sign(&fresh)),
        ],
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["text"], "Already handled.");

    // A captured request replayed ten minutes later is refused.
    let stale = (now().as_second() - 600).to_string();
    let (status, _) = send(
        &router,
        "/slack",
        &[
            ("X-Slack-Request-Timestamp", &stale),
            ("X-Slack-Signature", &sign(&stale)),
        ],
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A new command (new timestamp, so a new signature) is filed.
    let later = (now().as_second() + 1).to_string();
    let (status, json) = send(
        &router,
        "/slack",
        &[
            ("X-Slack-Request-Timestamp", &later),
            ("X-Slack-Signature", &sign(&later)),
        ],
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["text"], "Filed ENG-2 in triage.");
}

#[tokio::test]
async fn mail_files_issues_and_replies_become_comments() {
    let (router, _work) = setup(true).await;
    let mail = b"From: zoe@northstar.dev\nTo: ENG@intake.northstar.dev\n\
            Subject: Export fails\n\nNothing happens.\n"
        .to_vec();
    let (status, _) = send(
        &router,
        "/email",
        &[("Authorization", "Bearer wrong")],
        mail.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let bearer = [("Authorization", "Bearer mail-token")];
    let (status, json) = send(&router, "/email", &bearer, mail).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{json}");
    let issue = key(json["issue"].as_str().unwrap());

    let address =
        reply_address(REPLY, &issue, "bo", "bo@northstar.dev", "northstar.dev");
    let reply = format!(
        "From: bo@northstar.dev\nTo: {address}\nSubject: Re\n\nOn it.\n"
    );
    let (status, json) =
        send(&router, "/email", &bearer, reply.clone().into_bytes()).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{json}");
    assert_eq!(json["comment"], issue.to_string());

    // The same notification forwarded to someone else does not let them
    // comment as the user.
    let forwarded = reply.replace("From: bo@", "From: eve@");
    let (status, json) =
        send(&router, "/email", &bearer, forwarded.into_bytes()).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(json["ignored"], "reply address tag does not match");
}
