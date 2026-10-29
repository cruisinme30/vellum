// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Inputs and outputs of the `e2e` operations.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_kernel::{SpaceId, Timestamp, UserId};

/// Largest envelope accepted in one upload. The gateway's 10 `MiB` request
/// limit leaves room for the base64 encoding.
pub const MAX_ENVELOPE_BYTES: usize = 7 * 1024 * 1024;

/// Longest device label, in characters.
pub const MAX_LABEL_CHARS: usize = 80;

/// A space, by ID.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SpaceRef {
    /// The space.
    pub space_id: SpaceId,
}

/// Enrols the caller's device in a space.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EnrolDevice {
    /// The space.
    pub space_id: SpaceId,
    /// The device's X25519 public key, 64 hex digits, as
    /// `vellum private key` prints it.
    pub public_key: String,
    /// A name the members recognise, such as "Dana's laptop".
    pub label: String,
}

/// Revokes one device key.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RevokeDevice {
    /// The space.
    pub space_id: SpaceId,
    /// The key's fingerprint, 32 hex digits.
    pub fingerprint: String,
}

/// One enrolled device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeviceKeyInfo {
    /// The member the device belongs to.
    pub user_id: UserId,
    /// The X25519 public key, 64 hex digits.
    pub public_key: String,
    /// The first 16 bytes of SHA-256 of the key, 32 hex digits, as stored
    /// in envelope stanzas.
    pub fingerprint: String,
    /// The device's name.
    pub label: String,
    /// When it was enrolled.
    pub added_at: Timestamp,
    /// True for the caller's own devices.
    pub mine: bool,
}

/// A space's key list: what new pages are sealed to.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SpaceKeys {
    /// The space.
    pub space_id: SpaceId,
    /// Changes whenever the list changes; empty while the space has no
    /// keys (it is not private).
    pub version: String,
    /// Every enrolled device, oldest first.
    pub keys: Vec<DeviceKeyInfo>,
}

/// What a revocation removed. Members' devices strip these keys from
/// every envelope in the space (`vellum private reseal`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Revoked {
    /// The space.
    pub space_id: SpaceId,
    /// The removed keys.
    pub removed: Vec<DeviceKeyInfo>,
}

/// Lists a space's objects, a page at a time, in path order.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ListObjects {
    /// The space.
    pub space_id: SpaceId,
    /// Start after this path (the previous page's `next`).
    #[serde(default)]
    pub after: Option<String>,
    /// At most this many (default and maximum 1000).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One object in a space.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ObjectInfo {
    /// Its path inside the space folder.
    pub path: String,
    /// Its stored size in bytes.
    pub size: u64,
    /// When it last changed.
    pub modified: Timestamp,
}

/// A page of objects.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ObjectList {
    /// The objects, in path order.
    pub objects: Vec<ObjectInfo>,
    /// Pass as `after` for the next page; absent on the last one.
    pub next: Option<String>,
}

/// Downloads one object.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetObject {
    /// The space.
    pub space_id: SpaceId,
    /// Its path inside the space folder.
    pub path: String,
    /// Also return a readable page, so a member's device can seal it
    /// (needs edit access). Off by default: only ciphertext is served.
    #[serde(default)]
    pub plaintext: bool,
}

/// An object's ciphertext.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SealedObject {
    /// Its path inside the space folder.
    pub path: String,
    /// Its version, for `if_match` on the next write.
    pub etag: String,
    /// The recipient envelope (or, when not `sealed`, the readable
    /// bytes), base64.
    pub envelope: String,
    /// False for a readable object: the folder manifest, or a page still
    /// to convert when `plaintext` was asked for.
    pub sealed: bool,
}

/// Uploads a page sealed on the device.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PutObject {
    /// The space.
    pub space_id: SpaceId,
    /// Its path inside the space folder.
    pub path: String,
    /// The recipient envelope, base64. It must be sealed to exactly the
    /// space's current keys.
    pub envelope: String,
    /// The version this write replaces; absent to create a new object.
    #[serde(default)]
    pub if_match: Option<String>,
}

/// Deletes one object.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct DeleteObject {
    /// The space.
    pub space_id: SpaceId,
    /// Its path inside the space folder.
    pub path: String,
    /// The version being deleted.
    pub if_match: String,
}

/// A stored object.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Stored {
    /// Its path inside the space folder.
    pub path: String,
    /// Its new version.
    pub etag: String,
}

/// A deleted object.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Deleted {
    /// Its path inside the space folder.
    pub path: String,
}
