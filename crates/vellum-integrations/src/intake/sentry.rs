// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Sentry intake: new Sentry issues and alert-rule triggers become Vellum
//! issues.
//!
//! The Sentry integration signs each request with
//! `Sentry-Hook-Signature: <hex HMAC-SHA256 of the body>` keyed by the
//! integration's client secret, and names the resource in
//! `Sentry-Hook-Resource`. Only `issue.created` and
//! `event_alert.triggered` file issues; one Sentry issue files at most one
//! Vellum issue, keyed by Sentry's issue ID.

use std::collections::BTreeMap;

use serde_json::Value;

use super::Routing;
use super::text::{Body, Fact};
use crate::inbound::{InboundError, text};
use crate::mac;
use crate::model::{IssueDraft, Priority, Source, one_line};

/// The signature header.
pub const SIGNATURE_HEADER: &str = "Sentry-Hook-Signature";
/// The resource header.
pub const RESOURCE_HEADER: &str = "Sentry-Hook-Resource";

/// Verifies `Sentry-Hook-Signature`.
///
/// # Errors
///
/// [`InboundError::Unauthenticated`] if it is missing or wrong.
pub fn verify(
    client_secret: &[u8],
    signature: Option<&str>,
    body: &[u8],
) -> Result<(), InboundError> {
    match signature {
        Some(hex) if mac::verify_hex(client_secret, &[body], hex) => Ok(()),
        _ => Err(InboundError::Unauthenticated),
    }
}

fn priority(level: &str) -> Priority {
    match level {
        "fatal" => Priority::Urgent,
        "error" => Priority::High,
        "warning" => Priority::Normal,
        _ => Priority::Low,
    }
}

/// Turns a verified request into a draft; other resources and actions
/// return `None`. `project_teams` maps Sentry project slugs to team keys.
///
/// # Errors
///
/// [`InboundError::Malformed`] if the JSON is invalid or lacks the issue.
pub fn to_draft(
    resource: &str,
    body: &[u8],
    routing: &Routing,
    project_teams: &BTreeMap<String, String>,
) -> Result<Option<IssueDraft>, InboundError> {
    let json: Value =
        serde_json::from_slice(body).map_err(InboundError::malformed)?;
    let (title, culprit, level, link, issue_id, project, short_id) =
        match (resource, text(&json, "/action")) {
            ("issue", "created") => (
                text(&json, "/data/issue/title"),
                text(&json, "/data/issue/culprit"),
                text(&json, "/data/issue/level"),
                text(&json, "/data/issue/permalink"),
                text(&json, "/data/issue/id"),
                text(&json, "/data/issue/project/slug"),
                text(&json, "/data/issue/shortId"),
            ),
            ("event_alert", "triggered") => (
                text(&json, "/data/event/title"),
                text(&json, "/data/event/culprit"),
                text(&json, "/data/event/level"),
                text(&json, "/data/event/web_url"),
                text(&json, "/data/event/issue_id"),
                text(&json, "/data/event/project_slug"),
                "",
            ),
            _ => return Ok(None),
        };
    if issue_id.is_empty() || title.is_empty() {
        return Err(InboundError::malformed(
            "Sentry issue ID or title missing",
        ));
    }
    let team = project_teams
        .get(project)
        .and_then(|team| routing.team(team))
        .unwrap_or_else(|| routing.default_team.clone());
    let mut draft = IssueDraft::new(
        team,
        one_line(title, 255),
        Source::Sentry {
            issue_id: issue_id.to_owned(),
        },
    );
    draft.priority = Some(priority(level));
    let _ = draft.labels.insert("sentry".to_owned());
    draft.description = Body::new()
        .facts(&[
            Fact::new(
                "Level",
                if level.is_empty() { "unknown" } else { level },
            ),
            Fact::code("Culprit", culprit),
            Fact::new("Project", project),
            Fact::new("Sentry ID", short_id),
        ])
        .link("Open in Sentry", link)
        .finish();
    Ok(Some(draft))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn routing() -> Routing {
        Routing {
            default_team: "ENG".to_owned(),
            teams: ["ENG", "WEB"].iter().map(|t| (*t).to_owned()).collect(),
        }
    }

    fn issue_created() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "action": "created",
            "data": {"issue": {
                "id": "1170820242", "shortId": "WEB-3K", "title": "TypeError: \
                    Cannot read properties of undefined",
                "culprit": "checkout/cart.ts in total", "level": "error",
                "permalink": "https://northstar.sentry.io/issues/1170820242/",
                "project": {"slug": "web-app"}
            }}
        }))
        .unwrap_or_default()
    }

    #[test]
    fn verifies_the_signature() {
        let body = issue_created();
        let signature = mac::hex(&mac::hmac_sha256(b"client-secret", &[&body]));
        assert_eq!(verify(b"client-secret", Some(&signature), &body), Ok(()));
        assert_eq!(
            verify(b"other", Some(&signature), &body),
            Err(InboundError::Unauthenticated)
        );
        assert_eq!(
            verify(b"client-secret", None, &body),
            Err(InboundError::Unauthenticated)
        );
    }

    #[test]
    fn issue_created_files_a_draft() -> Result<(), InboundError> {
        let teams: BTreeMap<String, String> =
            std::iter::once(("web-app".to_owned(), "web".to_owned())).collect();
        let draft = to_draft("issue", &issue_created(), &routing(), &teams)?
            .ok_or_else(|| InboundError::malformed("no draft"))?;
        assert_eq!(draft.team, "WEB");
        assert_eq!(draft.priority, Some(Priority::High));
        assert!(draft.labels.contains("sentry"));
        assert!(
            draft
                .description
                .contains("- Culprit: `checkout/cart.ts in total`")
        );
        assert!(draft.description.contains(
            "[Open in Sentry](https://northstar.sentry.io/issues/1170820242/)"
        ));
        assert_eq!(
            draft.source,
            Source::Sentry {
                issue_id: "1170820242".to_owned()
            }
        );
        Ok(())
    }

    #[test]
    fn alerts_and_ignored_resources() -> Result<(), InboundError> {
        let alert = serde_json::to_vec(&json!({
            "action": "triggered",
            "data": {"event": {"title": "Out of memory", "level": "fatal", "issue_id": "42",
                               "web_url": "https://northstar.sentry.io/issues/4\
                                   2/events/x/", "project_slug": "api"}}
        }))
        .unwrap_or_default();
        let draft =
            to_draft("event_alert", &alert, &routing(), &BTreeMap::new())?
                .ok_or_else(|| InboundError::malformed("no draft"))?;
        assert_eq!(
            (draft.team.as_str(), draft.priority),
            ("ENG", Some(Priority::Urgent))
        );
        assert_eq!(
            to_draft(
                "issue",
                br#"{"action":"resolved"}"#,
                &routing(),
                &BTreeMap::new()
            )?,
            None
        );
        assert!(
            to_draft(
                "issue",
                br#"{"action":"created","data":{}}"#,
                &routing(),
                &BTreeMap::new()
            )
            .is_err()
        );
        Ok(())
    }
}
