// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The events that webhooks deliver.
//!
//! The brief (§8) lists the webhook resources: issue, comment, project,
//! project update, cycle, label, user and issue SLA. Every resource has
//! `create`, `update` and `remove`; the SLA resource has its own three
//! actions instead, because an SLA is never created or removed on its own.

use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A resource a webhook can subscribe to.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    /// An issue.
    Issue,
    /// A comment on an issue, project update or document.
    Comment,
    /// A project.
    Project,
    /// A project health update.
    ProjectUpdate,
    /// A team cycle.
    Cycle,
    /// An issue label.
    Label,
    /// A member of the organisation.
    User,
    /// An issue's SLA timer.
    IssueSla,
}

impl EventType {
    /// Every resource, in a stable order.
    pub const ALL: [Self; 8] = [
        Self::Issue,
        Self::Comment,
        Self::Project,
        Self::ProjectUpdate,
        Self::Cycle,
        Self::Label,
        Self::User,
        Self::IssueSla,
    ];

    /// The wire name, as in the payload's `type` field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Issue => "issue",
            Self::Comment => "comment",
            Self::Project => "project",
            Self::ProjectUpdate => "project_update",
            Self::Cycle => "cycle",
            Self::Label => "label",
            Self::User => "user",
            Self::IssueSla => "issue_sla",
        }
    }

    /// Whether `action` is valid for this resource.
    #[must_use]
    pub const fn allows(self, action: EventAction) -> bool {
        let sla_action = matches!(
            action,
            EventAction::SlaSet
                | EventAction::SlaHighRisk
                | EventAction::SlaBreached
        );
        match self {
            Self::IssueSla => sla_action,
            _ => !sla_action,
        }
    }
}

impl fmt::Display for EventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EventType {
    type Err = UnknownName;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|t| t.as_str() == s)
            .ok_or_else(|| UnknownName(s.to_owned()))
    }
}

/// What happened to the resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventAction {
    /// The resource was created.
    Create,
    /// The resource changed; the payload's `updated_from` holds old values.
    Update,
    /// The resource was deleted or archived.
    Remove,
    /// An SLA timer started or changed its deadline.
    SlaSet,
    /// An SLA is close to breaching (the orange band).
    SlaHighRisk,
    /// An SLA breached (the red band).
    SlaBreached,
}

impl EventAction {
    /// The wire name, as in the payload's `action` field.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Remove => "remove",
            Self::SlaSet => "sla_set",
            Self::SlaHighRisk => "sla_high_risk",
            Self::SlaBreached => "sla_breached",
        }
    }
}

/// A name that is not a known event type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown event type `{0}`")]
pub struct UnknownName(pub String);

/// Why an event cannot be published.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EventError {
    /// The action does not apply to the resource.
    #[error("action `{action}` is not valid for `{event_type}` events")]
    InvalidAction {
        /// The resource.
        event_type: EventType,
        /// The rejected action.
        action: &'static str,
    },
    /// The resource snapshot is not a JSON object.
    #[error("event data must be a JSON object")]
    DataNotObject,
}

/// One change, as the owning service publishes it.
///
/// `data` is the resource as the public API returns it, already filtered to
/// what any webhook may see; `team` scopes delivery to subscriptions that
/// include that team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookEvent {
    /// The event's unique ID (a ULID); receivers use it to de-duplicate.
    pub id: String,
    /// The resource.
    pub event_type: EventType,
    /// What happened.
    pub action: EventAction,
    /// When the change committed.
    pub occurred_at: Timestamp,
    /// The team key the resource belongs to, if any (`ENG`).
    pub team: Option<String>,
    /// The person or integration that made the change, by user ID.
    pub actor: Option<String>,
    /// The resource after the change.
    pub data: Value,
    /// Old values of the fields that changed, for `update`.
    pub updated_from: Option<Value>,
    /// A link to the resource in the web app.
    pub url: Option<String>,
}

impl WebhookEvent {
    /// Checks the action against the resource and the data shape.
    ///
    /// # Errors
    ///
    /// [`EventError`] when the action does not apply or `data` is not an
    /// object.
    pub fn validate(&self) -> Result<(), EventError> {
        if !self.event_type.allows(self.action) {
            return Err(EventError::InvalidAction {
                event_type: self.event_type,
                action: self.action.as_str(),
            });
        }
        if !self.data.is_object() {
            return Err(EventError::DataNotObject);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(event_type: EventType, action: EventAction) -> WebhookEvent {
        WebhookEvent {
            id: "01JD3Z6Q8W1R2T3Y4U5I6O7P8A".to_owned(),
            event_type,
            action,
            occurred_at: Timestamp::UNIX_EPOCH,
            team: Some("ENG".to_owned()),
            actor: None,
            data: json!({"id": "x"}),
            updated_from: None,
            url: None,
        }
    }

    #[test]
    fn names_round_trip() {
        for t in EventType::ALL {
            assert_eq!(t.as_str().parse::<EventType>(), Ok(t));
            assert_eq!(t.to_string(), t.as_str());
        }
        assert!("ticket".parse::<EventType>().is_err());
    }

    #[test]
    fn sla_actions_only_on_sla_events() {
        assert!(
            event(EventType::IssueSla, EventAction::SlaBreached)
                .validate()
                .is_ok()
        );
        assert!(
            event(EventType::IssueSla, EventAction::Create)
                .validate()
                .is_err()
        );
        assert!(
            event(EventType::Issue, EventAction::SlaSet)
                .validate()
                .is_err()
        );
        assert!(
            event(EventType::Comment, EventAction::Remove)
                .validate()
                .is_ok()
        );
    }

    #[test]
    fn data_must_be_an_object() {
        let mut e = event(EventType::Label, EventAction::Create);
        e.data = json!([1, 2]);
        assert_eq!(e.validate(), Err(EventError::DataNotObject));
    }
}
