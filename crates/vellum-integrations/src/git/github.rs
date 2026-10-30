// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! GitHub webhooks: `push`, `pull_request` and `pull_request_review`.
//!
//! The GitHub App (or repository webhook) signs each delivery with
//! `X-Hub-Signature-256: sha256=<hex HMAC-SHA256 of the body>`; the event
//! name is in `X-GitHub-Event`.

use serde_json::Value;

use super::event::{Commit, GitEvent, PrRef, Provider, PullRequestAction};
use crate::inbound::{InboundError, text};
use crate::mac;

/// The signature header.
pub const SIGNATURE_HEADER: &str = "X-Hub-Signature-256";
/// The event-name header.
pub const EVENT_HEADER: &str = "X-GitHub-Event";

/// Verifies `X-Hub-Signature-256` against the raw body.
///
/// # Errors
///
/// [`InboundError::Unauthenticated`] if it is missing or wrong.
pub fn verify(
    secret: &[u8],
    signature: Option<&str>,
    body: &[u8],
) -> Result<(), InboundError> {
    let hex = signature
        .and_then(|s| s.trim().strip_prefix("sha256="))
        .ok_or(InboundError::Unauthenticated)?;
    if mac::verify_hex(secret, &[body], hex) {
        Ok(())
    } else {
        Err(InboundError::Unauthenticated)
    }
}

/// Parses a verified delivery. Events and actions Vellum does not use
/// (labels, assignments, `synchronize` without changes) return `None`.
///
/// # Errors
///
/// [`InboundError::Malformed`] if the JSON is invalid.
pub fn parse(
    event: &str,
    body: &[u8],
) -> Result<Option<GitEvent>, InboundError> {
    let json: Value =
        serde_json::from_slice(body).map_err(InboundError::malformed)?;
    let repo = text(&json, "/repository/full_name").to_owned();
    if repo.is_empty() {
        return Err(InboundError::malformed("repository.full_name missing"));
    }
    match event {
        "push" => Ok(parse_push(&json, repo)),
        "pull_request" => {
            let action = match text(&json, "/action") {
                "opened" | "reopened" => {
                    if is_draft(&json) {
                        PullRequestAction::Draft
                    } else {
                        PullRequestAction::Opened
                    }
                }
                "converted_to_draft" => PullRequestAction::Draft,
                "ready_for_review" => PullRequestAction::ReadyForReview,
                "edited" | "synchronize" => PullRequestAction::Edited,
                "closed" => {
                    if json
                        .pointer("/pull_request/merged")
                        .and_then(Value::as_bool)
                        == Some(true)
                    {
                        PullRequestAction::Merged
                    } else {
                        PullRequestAction::ClosedUnmerged
                    }
                }
                _ => return Ok(None),
            };
            pull_request(&json, repo, action).map(Some)
        }
        "pull_request_review" => {
            if text(&json, "/action") != "submitted"
                && text(&json, "/action") != "dismissed"
            {
                return Ok(None);
            }
            let action =
                match (text(&json, "/action"), text(&json, "/review/state")) {
                    ("submitted", "approved") => PullRequestAction::Approved,
                    ("submitted", "changes_requested") | ("dismissed", _) => {
                        PullRequestAction::ChangesRequested
                    }
                    _ => return Ok(None),
                };
            pull_request(&json, repo, action).map(Some)
        }
        _ => Ok(None),
    }
}

fn is_draft(json: &Value) -> bool {
    json.pointer("/pull_request/draft").and_then(Value::as_bool) == Some(true)
}

fn parse_push(json: &Value, repo: String) -> Option<GitEvent> {
    let branch = text(json, "/ref").strip_prefix("refs/heads/")?.to_owned();
    if json.get("deleted").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    let default_branch = text(json, "/repository/default_branch") == branch;
    let commits = json
        .get("commits")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|c| Commit {
                    sha: text(c, "/id").to_owned(),
                    message: text(c, "/message").to_owned(),
                    url: c
                        .get("url")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(GitEvent::Push {
        provider: Provider::GitHub,
        repo,
        branch,
        default_branch,
        commits,
    })
}

fn pull_request(
    json: &Value,
    repo: String,
    action: PullRequestAction,
) -> Result<GitEvent, InboundError> {
    let number = json
        .pointer("/pull_request/number")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            InboundError::malformed("pull_request.number missing")
        })?;
    Ok(GitEvent::PullRequest {
        pr: PrRef {
            provider: Provider::GitHub,
            repo,
            number,
        },
        action,
        title: text(json, "/pull_request/title").to_owned(),
        body: text(json, "/pull_request/body").to_owned(),
        branch: text(json, "/pull_request/head/ref").to_owned(),
        url: text(json, "/pull_request/html_url").to_owned(),
        draft: is_draft(json),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body(value: &Value) -> Vec<u8> {
        serde_json::to_vec(value).unwrap_or_default()
    }

    #[test]
    fn verifies_signatures() {
        let payload = br#"{"zen":"Keep it logically awesome."}"#;
        let signature = format!(
            "sha256={}",
            mac::hex(&mac::hmac_sha256(b"s3cret", &[payload]))
        );
        assert_eq!(verify(b"s3cret", Some(&signature), payload), Ok(()));
        assert_eq!(
            verify(b"other", Some(&signature), payload),
            Err(InboundError::Unauthenticated)
        );
        assert_eq!(
            verify(b"s3cret", None, payload),
            Err(InboundError::Unauthenticated)
        );
        assert_eq!(
            verify(b"s3cret", Some("sha1=00"), payload),
            Err(InboundError::Unauthenticated)
        );
    }

    fn pr(action: &str, merged: bool, draft: bool) -> Value {
        json!({
            "action": action,
            "repository": {
                "full_name": "northstar/api",
                "default_branch": "main"
            },
            "pull_request": {
                "number": 12, "title": "ENG-142 Fix login",
                    "body": "Fixes ENG-143",
                "html_url": "https://github.com/northstar/api/pull/12",
                "draft": draft, "merged": merged, "head": {"ref": "ada/eng-142-\
                    fix-login"}
            },
            "review": {"state": "approved"}
        })
    }

    #[test]
    fn pull_request_actions() -> Result<(), InboundError> {
        let action =
            |event: &str,
             value: Value|
             -> Result<Option<PullRequestAction>, InboundError> {
                Ok(parse(event, &body(&value))?.and_then(|e| match e {
                    GitEvent::PullRequest { action, .. } => Some(action),
                    GitEvent::Push { .. } => None,
                }))
            };
        assert_eq!(
            action("pull_request", pr("opened", false, false))?,
            Some(PullRequestAction::Opened)
        );
        assert_eq!(
            action("pull_request", pr("opened", false, true))?,
            Some(PullRequestAction::Draft)
        );
        assert_eq!(
            action("pull_request", pr("closed", true, false))?,
            Some(PullRequestAction::Merged)
        );
        assert_eq!(
            action("pull_request", pr("closed", false, false))?,
            Some(PullRequestAction::ClosedUnmerged)
        );
        assert_eq!(action("pull_request", pr("labeled", false, false))?, None);
        assert_eq!(
            action("pull_request_review", pr("submitted", false, false))?,
            Some(PullRequestAction::Approved)
        );
        assert_eq!(action("issues", pr("opened", false, false))?, None);

        let event = parse("pull_request", &body(&pr("opened", false, false)))?;
        let Some(GitEvent::PullRequest {
            pr, branch, url, ..
        }) = event
        else {
            return Err(InboundError::malformed("expected a pull request"));
        };
        assert_eq!((pr.repo.as_str(), pr.number), ("northstar/api", 12));
        assert_eq!(branch, "ada/eng-142-fix-login");
        assert_eq!(url, "https://github.com/northstar/api/pull/12");
        Ok(())
    }

    #[test]
    fn pushes() -> Result<(), InboundError> {
        let push = json!({
            "ref": "refs/heads/main",
            "repository": {
                "full_name": "northstar/api",
                "default_branch": "main"
            },
            "commits": [{
                "id": "abc",
                "message": "Fix crash\n\nCloses ENG-9",
                "url": "https://github.com/c/abc"
            }]
        });
        let Some(GitEvent::Push {
            branch,
            default_branch,
            commits,
            ..
        }) = parse("push", &body(&push))?
        else {
            return Err(InboundError::malformed("expected a push"));
        };
        assert_eq!(branch, "main");
        assert!(default_branch);
        assert_eq!(commits.len(), 1);

        let tag =
            json!({"ref": "refs/tags/v1", "repository": {"full_name": "a/b"}});
        assert_eq!(parse("push", &body(&tag))?, None);
        assert!(parse("push", b"not json").is_err());
        assert!(parse("push", b"{}").is_err());
        Ok(())
    }
}
