// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Wire types of the push/pull contract (ADR 0023 D98, ADR 0029) and the
//! records the change log holds.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use vellum_kernel::{Error, ErrorKind, ObjectId, SpaceId};

/// Longest accepted client ID.
const CLIENT_ID_MAX: usize = 64;

/// One device's local store. Clients pick it (a UUID); the server binds
/// it to the first user who pushes with it.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, JsonSchema,
)]
#[serde(transparent)]
pub struct ClientId(String);

impl ClientId {
    /// Validates a client ID: 1 to 64 ASCII letters, digits or `-`.
    ///
    /// # Errors
    ///
    /// `InvalidInput` for anything else.
    pub fn parse(value: &str) -> Result<Self, Error> {
        let valid = !value.is_empty()
            && value.len() <= CLIENT_ID_MAX
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(Error::new(ErrorKind::InvalidInput, "invalid client id"))
        }
    }

    /// The ID as text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ClientId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ClientId {
    fn deserialize<D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw)
            .map_err(|err| serde::de::Error::custom(err.message().to_owned()))
    }
}

/// A mutation as the client sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClientMutation {
    /// Monotonic per client; the idempotency key.
    pub id: u64,
    /// Registered mutator name, for example `work.setState`.
    pub name: String,
    /// The mutator's arguments.
    pub args: Value,
    /// When the person made the change, epoch milliseconds (client clock).
    pub created_at: i64,
}

/// Body of `POST /api/v1/sync/push`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PushRequest {
    /// The pushing device.
    pub client_id: ClientId,
    /// Pending mutations, oldest first.
    pub mutations: Vec<ClientMutation>,
}

/// A mutation the server refused. It counts as processed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Rejected {
    /// The refused mutation.
    pub id: u64,
    /// Stable kernel error code, such as `permission_denied`.
    pub code: String,
    /// Caller-safe explanation.
    pub message: String,
}

/// Response to a push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PushResponse {
    /// Every mutation up to and including this ID has been processed.
    pub last_mutation_id: u64,
    /// The processed mutations that were refused.
    pub rejected: Vec<Rejected>,
}

/// Query of `GET /api/v1/sync/pull`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    /// The pulling device.
    pub client_id: ClientId,
    /// The cursor from the previous pull, empty on the first.
    #[serde(default)]
    pub cursor: String,
}

/// A change to one record, as the client applies it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Patch {
    /// The record now has this value.
    Put {
        /// Record family, such as `issues`.
        collection: String,
        /// Record key inside the collection.
        key: String,
        /// The whole new value.
        value: Value,
    },
    /// The record is gone (deleted, or no longer visible).
    Del {
        /// Record family.
        collection: String,
        /// Record key.
        key: String,
    },
}

/// Response to a pull.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PullResponse {
    /// Opaque; send it back on the next pull.
    pub cursor: String,
    /// The last mutation of this client that `patches` reflect.
    pub last_mutation_id: u64,
    /// When true the client drops all records before applying `patches`.
    pub reset: bool,
    /// Changes since the cursor, in order.
    pub patches: Vec<Patch>,
    /// When true, pull again at once with the new cursor.
    pub more: bool,
}

/// A change to a record, written by a mutator or a service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    /// The space the record belongs to; the shard key.
    pub space: SpaceId,
    /// The object whose read permission governs the record, when narrower
    /// than the space (a page, an issue).
    pub object: Option<ObjectId>,
    /// Record family.
    pub collection: String,
    /// Record key.
    pub key: String,
    /// The new value, or `None` for a delete.
    pub value: Option<Value>,
}

impl Change {
    /// A put.
    #[must_use]
    pub fn put(
        space: SpaceId,
        collection: &str,
        key: &str,
        value: Value,
    ) -> Self {
        Self {
            space,
            object: None,
            collection: collection.to_owned(),
            key: key.to_owned(),
            value: Some(value),
        }
    }

    /// A delete.
    #[must_use]
    pub fn delete(space: SpaceId, collection: &str, key: &str) -> Self {
        Self {
            space,
            object: None,
            collection: collection.to_owned(),
            key: key.to_owned(),
            value: None,
        }
    }

    /// Narrows read access to one object.
    #[must_use]
    pub const fn governed_by(mut self, object: ObjectId) -> Self {
        self.object = Some(object);
        self
    }

    /// The patch a reader with access receives.
    #[must_use]
    pub fn to_patch(&self) -> Patch {
        self.value.as_ref().map_or_else(
            || Patch::Del {
                collection: self.collection.clone(),
                key: self.key.clone(),
            },
            |value| Patch::Put {
                collection: self.collection.clone(),
                key: self.key.clone(),
                value: value.clone(),
            },
        )
    }
}

/// A change as stored: the latest version of one record and its position
/// in the log. Each record appears once, at its latest sequence number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The latest change.
    pub change: Change,
    /// Position in the log; increases on every write anywhere.
    pub seq: u64,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn client_ids_are_validated() {
        assert!(
            ClientId::parse("0b6f8c1e-91a2-4f63-9d1e-2b3c4d5e6f70").is_ok()
        );
        assert!(ClientId::parse("").is_err());
        assert!(ClientId::parse("has space").is_err());
        assert!(ClientId::parse(&"a".repeat(65)).is_err());
        let id = ClientId::parse("abc").unwrap_or_else(|_| unreachable!());
        assert_eq!(id.to_string(), "abc");
        assert!(serde_json::from_value::<ClientId>(json!("../etc")).is_err());
    }

    #[test]
    fn push_bodies_use_the_client_field_names() -> Result<(), serde_json::Error>
    {
        let request: PushRequest = serde_json::from_value(json!({
            "clientId": "c1",
            "mutations": [{
                "id": 1,
                "name": "work.setState",
                "args": {"key": "ENG-1"},
                "createdAt": 5
            }]
        }))?;
        assert_eq!(request.mutations.first().map(|m| m.created_at), Some(5));
        let response = serde_json::to_value(PushResponse {
            last_mutation_id: 1,
            rejected: vec![],
        })?;
        assert_eq!(response, json!({"lastMutationId": 1, "rejected": []}));
        Ok(())
    }

    #[test]
    fn patches_are_tagged_by_op() -> Result<(), serde_json::Error> {
        let space = SpaceId::generate();
        let put =
            Change::put(space, "issues", "ENG-1", json!({"t": 1})).to_patch();
        let del = Change::delete(space, "issues", "ENG-1").to_patch();
        assert_eq!(
            serde_json::to_value(put)?,
            json!({
                "op": "put",
                "collection": "issues",
                "key": "ENG-1",
                "value": {"t": 1}
            })
        );
        assert_eq!(
            serde_json::to_value(del)?,
            json!({"op": "del", "collection": "issues", "key": "ENG-1"})
        );
        let pull: PullRequest =
            serde_json::from_value(json!({"clientId": "c"}))?;
        assert_eq!(pull.cursor, "");
        Ok(())
    }
}
