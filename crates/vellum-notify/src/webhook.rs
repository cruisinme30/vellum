// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Webhook subscriptions and the signed requests they receive.
//!
//! An admin creates a subscription with a URL, the resources it wants and
//! the teams it covers. For each [`WebhookEvent`] the dispatcher selects
//! the matching subscriptions and builds one [`OutboundRequest`] each. The
//! body is the [`WebhookPayload`] as JSON; the signature header covers the
//! exact bytes sent.

use std::collections::BTreeSet;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::event::{EventAction, EventType, WebhookEvent};
use crate::secret::WebhookSecret;
use crate::signature::{SIGNATURE_HEADER, sign};

/// The header naming the resource, for routing without parsing the body.
pub const EVENT_HEADER: &str = "Vellum-Event";
/// The header carrying the delivery ID, stable across retries.
pub const DELIVERY_HEADER: &str = "Vellum-Delivery";
/// The user agent of every webhook request.
pub const USER_AGENT: &str = concat!("Vellum-Webhook/", env!("CARGO_PKG_VERSION"));

/// Which teams a subscription hears about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "teams", rename_all = "snake_case")]
pub enum TeamScope {
    /// Every public team, including teams created later. Private teams are
    /// never covered implicitly.
    AllPublic,
    /// Exactly these team keys, private ones included.
    Only(BTreeSet<String>),
}

/// A webhook an admin created.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    /// The subscription's ID.
    pub id: String,
    /// The receiver; already passed [`crate::endpoint::check_url`].
    pub url: Url,
    /// The current secret first, then the previous one while it rotates out.
    pub secrets: Vec<WebhookSecret>,
    /// The resources it wants.
    pub event_types: BTreeSet<EventType>,
    /// The teams it covers.
    pub teams: TeamScope,
    /// Paused hooks receive nothing; Vellum pauses a hook that keeps failing.
    pub enabled: bool,
}

impl Subscription {
    /// Whether this subscription should receive `event`.
    ///
    /// Events without a team (users, organisation labels) go to every
    /// subscription that wants the resource. `private_teams` lists the keys
    /// of private teams, which only an explicit [`TeamScope::Only`] covers.
    #[must_use]
    pub fn matches(&self, event: &WebhookEvent, private_teams: &BTreeSet<String>) -> bool {
        if !self.enabled || !self.event_types.contains(&event.event_type) {
            return false;
        }
        match (&event.team, &self.teams) {
            (None, _) => true,
            (Some(team), TeamScope::AllPublic) => !private_teams.contains(team),
            (Some(team), TeamScope::Only(keys)) => keys.contains(team),
        }
    }
}

/// The JSON body of a webhook request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookPayload {
    /// The event ID; the same event sent to two hooks has the same ID.
    pub id: String,
    /// The resource.
    #[serde(rename = "type")]
    pub event_type: EventType,
    /// What happened.
    pub action: EventAction,
    /// When the change committed (RFC 3339).
    pub created_at: Timestamp,
    /// The team key, when the resource belongs to a team.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    /// The user who made the change.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    /// The resource after the change.
    pub data: Value,
    /// Old values of changed fields, for `update`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_from: Option<Value>,
    /// Link to the resource in the web app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The receiving subscription.
    pub webhook_id: String,
    /// When this attempt was signed (RFC 3339); also in the signature.
    pub webhook_timestamp: Timestamp,
}

/// An HTTP request ready for a transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundRequest {
    /// Where to send it.
    pub url: Url,
    /// Header names and values, in order.
    pub headers: Vec<(String, String)>,
    /// The exact bytes that were signed.
    pub body: Vec<u8>,
    /// Stable across retries, so receivers can de-duplicate.
    pub delivery_id: String,
}

impl OutboundRequest {
    /// The value of the first header with this name (case-insensitive).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Builds the signed request for one subscription, signed at `now`.
///
/// # Errors
///
/// [`serde_json::Error`] if the payload cannot be serialised, which only
/// happens for non-string map keys in `data`.
pub fn prepare(
    event: &WebhookEvent,
    subscription: &Subscription,
    now: Timestamp,
) -> Result<OutboundRequest, serde_json::Error> {
    let payload = WebhookPayload {
        id: event.id.clone(),
        event_type: event.event_type,
        action: event.action,
        created_at: event.occurred_at,
        team: event.team.clone(),
        actor: event.actor.clone(),
        data: event.data.clone(),
        updated_from: event.updated_from.clone(),
        url: event.url.clone(),
        webhook_id: subscription.id.clone(),
        webhook_timestamp: now,
    };
    let body = serde_json::to_vec(&payload)?;
    let secrets: Vec<&WebhookSecret> = subscription.secrets.iter().collect();
    let delivery_id = format!("{}.{}", event.id, subscription.id);
    let headers = vec![
        (
            "Content-Type".to_owned(),
            "application/json; charset=utf-8".to_owned(),
        ),
        ("User-Agent".to_owned(), USER_AGENT.to_owned()),
        (
            EVENT_HEADER.to_owned(),
            event.event_type.as_str().to_owned(),
        ),
        (DELIVERY_HEADER.to_owned(), delivery_id.clone()),
        (SIGNATURE_HEADER.to_owned(), sign(&secrets, now, &body)),
    ];
    Ok(OutboundRequest {
        url: subscription.url.clone(),
        headers,
        body,
        delivery_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signature::{DEFAULT_TOLERANCE, verify};
    use serde_json::json;

    fn subscription(teams: TeamScope) -> Subscription {
        Subscription {
            id: "hook1".to_owned(),
            url: Url::parse("https://hooks.northstar.dev/v").unwrap_or_else(|_| unreachable!()),
            secrets: vec![WebhookSecret::from_bytes([3; 32])],
            event_types: [EventType::Issue, EventType::User].into_iter().collect(),
            teams,
            enabled: true,
        }
    }

    fn event(event_type: EventType, team: Option<&str>) -> WebhookEvent {
        WebhookEvent {
            id: "evt1".to_owned(),
            event_type,
            action: EventAction::Update,
            occurred_at: Timestamp::UNIX_EPOCH,
            team: team.map(str::to_owned),
            actor: Some("user1".to_owned()),
            data: json!({"identifier": "ENG-142", "title": "Fix login"}),
            updated_from: Some(json!({"title": "Fix logn"})),
            url: Some("https://vellum.northstar.dev/issue/ENG-142".to_owned()),
        }
    }

    #[test]
    fn matching_respects_types_teams_and_privacy() {
        let private: BTreeSet<String> = std::iter::once("SEC".to_owned()).collect();
        let all = subscription(TeamScope::AllPublic);
        assert!(all.matches(&event(EventType::Issue, Some("ENG")), &private));
        assert!(!all.matches(&event(EventType::Issue, Some("SEC")), &private));
        assert!(!all.matches(&event(EventType::Cycle, Some("ENG")), &private));
        assert!(all.matches(&event(EventType::User, None), &private));

        let only = subscription(TeamScope::Only(std::iter::once("SEC".to_owned()).collect()));
        assert!(only.matches(&event(EventType::Issue, Some("SEC")), &private));
        assert!(!only.matches(&event(EventType::Issue, Some("ENG")), &private));

        let mut paused = subscription(TeamScope::AllPublic);
        paused.enabled = false;
        assert!(!paused.matches(&event(EventType::Issue, Some("ENG")), &private));
    }

    #[test]
    fn prepared_request_is_signed_over_the_body() -> Result<(), serde_json::Error> {
        let sub = subscription(TeamScope::AllPublic);
        let now = Timestamp::from_second(1_800_000_000).unwrap_or(Timestamp::UNIX_EPOCH);
        let request = prepare(&event(EventType::Issue, Some("ENG")), &sub, now)?;
        assert_eq!(request.delivery_id, "evt1.hook1");
        assert_eq!(request.header("vellum-event"), Some("issue"));
        let header = request.header(SIGNATURE_HEADER).unwrap_or_default();
        let secret = sub.secrets.first().unwrap_or_else(|| unreachable!());
        assert_eq!(
            verify(header, &request.body, secret, now, DEFAULT_TOLERANCE),
            Ok(())
        );

        let body: Value = serde_json::from_slice(&request.body)?;
        assert_eq!(body["type"], "issue");
        assert_eq!(body["action"], "update");
        assert_eq!(body["team"], "ENG");
        assert_eq!(body["updated_from"]["title"], "Fix logn");
        assert_eq!(body["webhook_id"], "hook1");
        assert_eq!(body["created_at"], "1970-01-01T00:00:00Z");
        Ok(())
    }
}
