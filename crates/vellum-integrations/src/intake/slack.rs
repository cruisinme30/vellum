// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Slack intake: the `/vellum` slash command and the "Create issue"
//! message action.
//!
//! Slack signs each request with `X-Slack-Signature: v0=<hex HMAC-SHA256
//! of "v0:{timestamp}:{body}">` and sends the timestamp in
//! `X-Slack-Request-Timestamp`; requests older than five minutes are
//! refused to stop replays.
//!
//! `/vellum [TEAM] <title> [p1…p4] [#label …]` files an issue: a first
//! word that is a team key picks the team, `p1`–`p4` set the priority and
//! `#words` add labels. The message action files the message itself, with
//! a link back to it.

use jiff::{SignedDuration, Timestamp};
use serde_json::Value;

use super::Routing;
use super::text::Body;
use crate::inbound::{InboundError, text};
use crate::mac;
use crate::model::{IssueDraft, Priority, Source, one_line};

/// The signature header.
pub const SIGNATURE_HEADER: &str = "X-Slack-Signature";
/// The timestamp header.
pub const TIMESTAMP_HEADER: &str = "X-Slack-Request-Timestamp";

/// Verifies a Slack request.
///
/// # Errors
///
/// [`InboundError::Unauthenticated`] if the signature is missing, wrong or
/// older than five minutes.
pub fn verify(
    signing_secret: &[u8],
    timestamp: Option<&str>,
    signature: Option<&str>,
    body: &[u8],
    now: Timestamp,
) -> Result<(), InboundError> {
    let ts = timestamp.ok_or(InboundError::Unauthenticated)?.trim();
    let seconds: i64 = ts.parse().map_err(|_| InboundError::Unauthenticated)?;
    let sent = Timestamp::from_second(seconds).map_err(|_| InboundError::Unauthenticated)?;
    if now.duration_since(sent).abs() > SignedDuration::from_mins(5) {
        return Err(InboundError::Unauthenticated);
    }
    let hex = signature
        .and_then(|s| s.trim().strip_prefix("v0="))
        .ok_or(InboundError::Unauthenticated)?;
    if mac::verify_hex(signing_secret, &[b"v0:", ts.as_bytes(), b":", body], hex) {
        Ok(())
    } else {
        Err(InboundError::Unauthenticated)
    }
}

/// A verified Slack request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlackRequest {
    /// `/vellum …`.
    Command {
        /// The text after the command.
        text: String,
        /// The Slack user ID.
        user_id: String,
        /// Where to post the confirmation.
        response_url: String,
    },
    /// "Create issue" on a message.
    MessageAction {
        /// The message text.
        text: String,
        /// The Slack user who ran the action.
        user_id: String,
        /// A permalink to the message.
        permalink: String,
        /// Where to post the confirmation.
        response_url: String,
    },
}

/// Parses a verified, form-encoded Slack request body.
///
/// # Errors
///
/// [`InboundError::Malformed`] for anything that is neither a slash command
/// nor a message action.
pub fn parse(body: &[u8]) -> Result<SlackRequest, InboundError> {
    let mut fields = std::collections::BTreeMap::new();
    for (key, value) in url::form_urlencoded::parse(body) {
        let _ = fields.insert(key.into_owned(), value.into_owned());
    }
    let field = |name: &str| fields.get(name).cloned().unwrap_or_default();
    if let Some(payload) = fields.get("payload") {
        let json: Value = serde_json::from_str(payload).map_err(InboundError::malformed)?;
        if text(&json, "/type") != "message_action" {
            return Err(InboundError::malformed("unsupported interaction type"));
        }
        let domain = text(&json, "/team/domain");
        let channel = text(&json, "/channel/id");
        let ts = text(&json, "/message/ts").replace('.', "");
        return Ok(SlackRequest::MessageAction {
            text: text(&json, "/message/text").to_owned(),
            user_id: text(&json, "/user/id").to_owned(),
            permalink: format!("https://{domain}.slack.com/archives/{channel}/p{ts}"),
            response_url: text(&json, "/response_url").to_owned(),
        });
    }
    if field("command").is_empty() {
        return Err(InboundError::malformed(
            "neither a command nor an interaction",
        ));
    }
    Ok(SlackRequest::Command {
        text: field("text"),
        user_id: field("user_id"),
        response_url: field("response_url"),
    })
}

/// Turns a request into a draft. `reporter` is the Vellum user linked to
/// the Slack user, when there is one. Returns `None` for an empty command
/// (the receiver replies with usage help).
#[must_use]
pub fn to_draft(
    request: &SlackRequest,
    routing: &Routing,
    reporter: Option<String>,
) -> Option<IssueDraft> {
    match request {
        SlackRequest::Command { text, .. } => {
            let mut words: Vec<&str> = text.split_whitespace().collect();
            let named = words.first().and_then(|w| routing.team(w));
            if named.is_some() {
                let _ = words.remove(0);
            }
            let team = named.unwrap_or_else(|| routing.default_team.clone());
            let mut draft = IssueDraft::new(team, String::new(), Source::Slack { permalink: None });
            let mut title = Vec::new();
            for word in words {
                if let Some(label) = word.strip_prefix('#').filter(|l| !l.is_empty()) {
                    let _ = draft.labels.insert(label.to_owned());
                } else if word.len() == 2
                    && word.starts_with(['p', 'P'])
                    && let Some(priority) = Priority::parse(word)
                {
                    draft.priority = Some(priority);
                } else {
                    title.push(word);
                }
            }
            draft.title = one_line(&title.join(" "), 255);
            draft.reporter = reporter;
            (!draft.title.is_empty()).then_some(draft)
        }
        SlackRequest::MessageAction {
            text, permalink, ..
        } => {
            let title = one_line(text, 120);
            if title.is_empty() {
                return None;
            }
            let mut draft = IssueDraft::new(
                routing.default_team.clone(),
                title,
                Source::Slack {
                    permalink: Some(permalink.clone()),
                },
            );
            draft.description = Body::new()
                .quote(text)
                .link("View in Slack", permalink)
                .finish();
            draft.reporter = reporter;
            Some(draft)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn routing() -> Routing {
        Routing {
            default_team: "ENG".to_owned(),
            teams: ["ENG", "OPS"].iter().map(|t| (*t).to_owned()).collect(),
        }
    }

    #[test]
    fn verifies_signed_requests() {
        // The example from Slack's "Verifying requests" guide.
        let secret = b"8f742231b10e8888abcd99yyyzzz85a5";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J&team_domain=testteamnow&channel_id=G8PSS9T3V&channel_name=foobar&user_id=U2CERLKJA&user_name=roadrunner&command=%2Fwebhook-collect&text=&response_url=https%3A%2F%2Fhooks.slack.com%2Fcommands%2FT1DC2JH3J%2F397700885554%2F96rGlfmibIGlgcZRskXaIFfN&trigger_id=398738663015.47445629121.803a0bc887a14d10d2c447fce8b6703c";
        let signature = "v0=a2114d57b48eac39b9ad189dd8316235a7b4a8d21a10bd27519666489c69b503";
        let now = Timestamp::from_second(1_531_420_618).unwrap_or(Timestamp::UNIX_EPOCH);
        assert_eq!(
            verify(secret, Some("1531420618"), Some(signature), body, now),
            Ok(())
        );
        assert_eq!(
            verify(
                secret,
                Some("1531420618"),
                Some(signature),
                b"tampered",
                now
            ),
            Err(InboundError::Unauthenticated)
        );
        let later = Timestamp::from_second(1_531_421_000).unwrap_or(Timestamp::UNIX_EPOCH);
        assert_eq!(
            verify(secret, Some("1531420618"), Some(signature), body, later),
            Err(InboundError::Unauthenticated)
        );
        assert!(verify(secret, None, Some(signature), body, now).is_err());
    }

    #[test]
    fn slash_command_to_draft() -> Result<(), InboundError> {
        let body = b"command=%2Fvellum&text=ops+Disk+full+on+db-2+p1+%23infra+%23pager&user_id=U1&response_url=https%3A%2F%2Fhooks.slack.com%2Fx";
        let request = parse(body)?;
        let draft = to_draft(&request, &routing(), Some("ada".to_owned()))
            .ok_or_else(|| InboundError::malformed("no draft"))?;
        assert_eq!(draft.team, "OPS");
        assert_eq!(draft.title, "Disk full on db-2");
        assert_eq!(draft.priority, Some(Priority::Urgent));
        assert_eq!(
            draft.labels.iter().map(String::as_str).collect::<Vec<_>>(),
            ["infra", "pager"]
        );
        assert_eq!(draft.reporter.as_deref(), Some("ada"));

        let empty = parse(b"command=%2Fvellum&text=&user_id=U1")?;
        assert_eq!(to_draft(&empty, &routing(), None), None);
        Ok(())
    }

    #[test]
    fn message_action_to_draft() -> Result<(), InboundError> {
        let payload = serde_json::json!({
            "type": "message_action",
            "team": {"domain": "northstar"},
            "channel": {"id": "C024BE91L"},
            "user": {"id": "U1"},
            "message": {"text": "Checkout is throwing 500s\nsince the deploy", "ts": "1700000000.000100"},
            "response_url": "https://hooks.slack.com/app/x"
        });
        let body = format!(
            "payload={}",
            url::form_urlencoded::byte_serialize(payload.to_string().as_bytes())
                .collect::<String>()
        );
        let request = parse(body.as_bytes())?;
        let draft = to_draft(&request, &routing(), None)
            .ok_or_else(|| InboundError::malformed("no draft"))?;
        assert_eq!(draft.team, "ENG");
        assert_eq!(draft.title, "Checkout is throwing 500s");
        assert!(draft.description.contains("> since the deploy"));
        assert_eq!(
            draft.source,
            Source::Slack {
                permalink: Some(
                    "https://northstar.slack.com/archives/C024BE91L/p1700000000000100".to_owned()
                )
            }
        );
        assert!(parse(b"payload=%7B%22type%22%3A%22view_submission%22%7D").is_err());
        assert!(parse(b"text=hi").is_err());
        Ok(())
    }
}
