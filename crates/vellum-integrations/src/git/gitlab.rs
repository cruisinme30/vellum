// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! GitLab webhooks: `Push Hook` and `Merge Request Hook`.
//!
//! GitLab authenticates deliveries with the secret token configured on
//! the hook, sent back verbatim in `X-Gitlab-Token`; Vellum compares it in
//! constant time. The event name is in `X-Gitlab-Event`.

use serde_json::Value;

use super::event::{Commit, GitEvent, PrRef, Provider, PullRequestAction};
use crate::inbound::{InboundError, text};
use crate::mac;

/// The token header.
pub const TOKEN_HEADER: &str = "X-Gitlab-Token";
/// The event-name header.
pub const EVENT_HEADER: &str = "X-Gitlab-Event";

/// Verifies `X-Gitlab-Token`.
///
/// # Errors
///
/// [`InboundError::Unauthenticated`] if it is missing or wrong.
pub fn verify(token: &str, header: Option<&str>) -> Result<(), InboundError> {
    match header {
        Some(given)
            if !token.is_empty()
                && mac::equal(given.as_bytes(), token.as_bytes()) =>
        {
            Ok(())
        }
        _ => Err(InboundError::Unauthenticated),
    }
}

/// Parses a verified delivery; unused events and actions return `None`.
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
    let repo = text(&json, "/project/path_with_namespace").to_owned();
    if repo.is_empty() {
        return Err(InboundError::malformed(
            "project.path_with_namespace missing",
        ));
    }
    match event {
        "Push Hook" => Ok(parse_push(&json, repo)),
        "Merge Request Hook" => parse_merge_request(&json, repo),
        _ => Ok(None),
    }
}

fn parse_push(json: &Value, repo: String) -> Option<GitEvent> {
    let branch = text(json, "/ref").strip_prefix("refs/heads/")?.to_owned();
    if text(json, "/after").bytes().all(|b| b == b'0') {
        return None; // branch deleted
    }
    let default_branch = text(json, "/project/default_branch") == branch;
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
        provider: Provider::GitLab,
        repo,
        branch,
        default_branch,
        commits,
    })
}

fn parse_merge_request(
    json: &Value,
    repo: String,
) -> Result<Option<GitEvent>, InboundError> {
    let attrs = json
        .get("object_attributes")
        .ok_or_else(|| InboundError::malformed("object_attributes missing"))?;
    let draft = attrs
        .get("draft")
        .or_else(|| attrs.get("work_in_progress"))
        .and_then(Value::as_bool)
        == Some(true);
    let draft_changed = json
        .pointer("/changes/draft/current")
        .and_then(Value::as_bool);
    let action = match (text(attrs, "/action"), draft_changed) {
        ("open" | "reopen", _) if draft => PullRequestAction::Draft,
        ("open" | "reopen", _) => PullRequestAction::Opened,
        ("update", Some(true)) => PullRequestAction::Draft,
        ("update", Some(false)) => PullRequestAction::ReadyForReview,
        ("update", None) => PullRequestAction::Edited,
        ("approved" | "approval", _) => PullRequestAction::Approved,
        ("unapproved" | "unapproval", _) => PullRequestAction::ChangesRequested,
        ("merge", _) => PullRequestAction::Merged,
        ("close", _) => PullRequestAction::ClosedUnmerged,
        _ => return Ok(None),
    };
    let number = attrs.get("iid").and_then(Value::as_u64).ok_or_else(|| {
        InboundError::malformed("object_attributes.iid missing")
    })?;
    Ok(Some(GitEvent::PullRequest {
        pr: PrRef {
            provider: Provider::GitLab,
            repo,
            number,
        },
        action,
        title: text(attrs, "/title").to_owned(),
        body: text(attrs, "/description").to_owned(),
        branch: text(attrs, "/source_branch").to_owned(),
        url: text(attrs, "/url").to_owned(),
        draft,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body(value: &Value) -> Vec<u8> {
        serde_json::to_vec(value).unwrap_or_default()
    }

    #[test]
    fn verifies_the_token() {
        assert_eq!(verify("t0ken", Some("t0ken")), Ok(()));
        assert_eq!(
            verify("t0ken", Some("t0ke")),
            Err(InboundError::Unauthenticated)
        );
        assert_eq!(verify("t0ken", None), Err(InboundError::Unauthenticated));
        assert_eq!(verify("", Some("")), Err(InboundError::Unauthenticated));
    }

    fn mr(action: &str, extra: &Value) -> Value {
        let mut value = json!({
            "project": {
                "path_with_namespace": "northstar/web",
                "default_branch": "main"
            },
            "object_attributes": {
                "iid": 5, "action": action, "title": "ENG-9 Speed up",
                    "description": "",
                "url": "https://gitlab.northstar.dev/northstar/web/-/merge_requ\
                    ests/5",
                "source_branch": "bo/eng-9-speed-up", "draft": false
            }
        });
        if let (Some(target), Some(extra)) =
            (value.as_object_mut(), extra.as_object())
        {
            for (k, v) in extra {
                let _ = target.insert(k.clone(), v.clone());
            }
        }
        value
    }

    fn action(value: &Value) -> Option<PullRequestAction> {
        match parse("Merge Request Hook", &body(value)) {
            Ok(Some(GitEvent::PullRequest { action, .. })) => Some(action),
            _ => None,
        }
    }

    #[test]
    fn merge_request_actions() {
        let none = json!({});
        assert_eq!(action(&mr("open", &none)), Some(PullRequestAction::Opened));
        assert_eq!(
            action(&mr("approved", &none)),
            Some(PullRequestAction::Approved)
        );
        assert_eq!(
            action(&mr("merge", &none)),
            Some(PullRequestAction::Merged)
        );
        assert_eq!(
            action(&mr("close", &none)),
            Some(PullRequestAction::ClosedUnmerged)
        );
        assert_eq!(
            action(&mr("update", &none)),
            Some(PullRequestAction::Edited)
        );
        let to_draft =
            json!({"changes": {"draft": {"previous": false, "current": true}}});
        assert_eq!(
            action(&mr("update", &to_draft)),
            Some(PullRequestAction::Draft)
        );
        let ready =
            json!({"changes": {"draft": {"previous": true, "current": false}}});
        assert_eq!(
            action(&mr("update", &ready)),
            Some(PullRequestAction::ReadyForReview)
        );
        assert_eq!(action(&mr("unknown", &none)), None);
    }

    #[test]
    fn pushes_and_deletions() -> Result<(), InboundError> {
        let push = json!({
            "ref": "refs/heads/bo/eng-9-speed-up", "after": "da1560886d4f094c3e\
                6c9ef40349f7d38b5d27d7",
            "project": {
                "path_with_namespace": "northstar/web",
                "default_branch": "main"
            },
            "commits": [{"id": "da15", "message": "Speed up", "url": "https://g\
                itlab.northstar.dev/c"}]
        });
        let event = parse("Push Hook", &body(&push))?;
        assert!(matches!(
            event,
            Some(GitEvent::Push {
                default_branch: false,
                ..
            })
        ));
        let deleted = json!({
            "ref": "refs/heads/x", "after": "0000000000000000000000000000000000\
                000000",
            "project": {"path_with_namespace": "northstar/web"}
        });
        assert_eq!(parse("Push Hook", &body(&deleted))?, None);
        assert_eq!(parse("Note Hook", &body(&deleted))?, None);
        Ok(())
    }
}
