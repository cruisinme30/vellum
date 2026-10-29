// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! End-to-end encrypted private spaces (D25, R-SE-10): the member key
//! registry and what the server may do with an E2E space.
//!
//! In an E2E space every document is a recipient envelope
//! (`vellum_crypto::recipients`), sealed on a member's device to the
//! X25519 public keys of all member devices. The server stores the
//! ciphertext and this registry of public keys; it never sees a secret
//! key or a plaintext.
//!
//! [`KeyRegistry`] answers "who must a new document be sealed to"
//! ([`KeyRegistry::recipients`]) and, when a device or member is removed,
//! which keys clients must strip from existing envelopes
//! ([`Removed`]). Stripping a stanza stops future access only: a removed
//! member who kept old ciphertext and their key can still read what they
//! could read before. New writes use fresh file keys, so later edits stay
//! private.
//!
//! [`ServerFeature`] lists the server-side features that need plaintext;
//! [`SpaceMode::allows`] refuses all of them for E2E spaces. Search and
//! MCP for those spaces run on the desktop client instead.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};
use vellum_kernel::{SpaceId, UserId};

/// Length of an X25519 public key.
pub const KEY_LEN: usize = 32;

/// Length of a key fingerprint, matching `vellum_crypto::recipients`.
pub const FINGERPRINT_LEN: usize = 16;

/// Most recipients one envelope can carry, matching
/// `vellum_crypto::recipients::MAX_RECIPIENTS`.
pub const MAX_RECIPIENTS: usize = 255;

/// Encodings of the X25519 points of small order (and their non-canonical
/// twins). A shared secret computed with one of these is predictable, so
/// they are refused at enrolment.
const SMALL_ORDER: [[u8; KEY_LEN]; 7] = [
    [0; KEY_LEN],
    {
        let mut k = [0; KEY_LEN];
        k[0] = 1;
        k
    },
    [
        0xe0, 0xeb, 0x7a, 0x7c, 0x3b, 0x41, 0xb8, 0xae, 0x16, 0x56, 0xe3, 0xfa, 0xf1, 0x9f, 0xc4,
        0x6a, 0xda, 0x09, 0x8d, 0xeb, 0x9c, 0x32, 0xb1, 0xfd, 0x86, 0x62, 0x05, 0x16, 0x5f, 0x49,
        0xb8, 0x00,
    ],
    [
        0x5f, 0x9c, 0x95, 0xbc, 0xa3, 0x50, 0x8c, 0x24, 0xb1, 0xd0, 0xb1, 0x55, 0x9c, 0x83, 0xef,
        0x5b, 0x04, 0x44, 0x5c, 0xc4, 0x58, 0x1c, 0x8e, 0x86, 0xd8, 0x22, 0x4e, 0xdd, 0xd0, 0x9f,
        0x11, 0x57,
    ],
    [
        0xec, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x7f,
    ],
    [
        0xed, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x7f,
    ],
    [
        0xee, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x7f,
    ],
];

/// A member device's X25519 public key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceKey([u8; KEY_LEN]);

impl DeviceKey {
    /// Validates a public key sent by a client.
    ///
    /// # Errors
    ///
    /// [`E2eError::WeakKey`] for a small-order point.
    pub fn new(bytes: [u8; KEY_LEN]) -> Result<Self, E2eError> {
        // Bit 255 is ignored by X25519, so compare with it cleared too.
        let mut masked = bytes;
        if let Some(last) = masked.last_mut() {
            *last &= 0x7f;
        }
        if SMALL_ORDER
            .iter()
            .any(|weak| *weak == bytes || *weak == masked)
        {
            return Err(E2eError::WeakKey);
        }
        Ok(Self(bytes))
    }

    /// The raw key, for `vellum_crypto::recipients::RecipientPublic`.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; KEY_LEN] {
        self.0
    }

    /// The first 16 bytes of SHA-256 of the key, as stored in envelope
    /// stanzas.
    #[must_use]
    pub fn fingerprint(self) -> [u8; FINGERPRINT_LEN] {
        let digest = Sha256::digest(self.0);
        let mut out = [0_u8; FINGERPRINT_LEN];
        for (slot, byte) in out.iter_mut().zip(digest.iter()) {
            *slot = *byte;
        }
        out
    }
}

/// Why a registry change was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum E2eError {
    /// The key is a small-order point.
    #[error("public key is not a valid X25519 key")]
    WeakKey,
    /// The key is already enrolled, for this or another member.
    #[error("public key is already enrolled")]
    Duplicate,
    /// The space would exceed [`MAX_RECIPIENTS`] device keys.
    #[error("space already has the maximum of {MAX_RECIPIENTS} device keys")]
    TooManyKeys,
    /// Removing this would leave the space with no key, so nobody could
    /// read it again.
    #[error("cannot remove the last key of a space")]
    LastKey,
    /// The space, member or key is not in the registry.
    #[error("not enrolled")]
    NotEnrolled,
}

impl From<E2eError> for vellum_kernel::Error {
    fn from(err: E2eError) -> Self {
        let kind = match err {
            E2eError::NotEnrolled => vellum_kernel::ErrorKind::NotFound,
            E2eError::Duplicate | E2eError::LastKey | E2eError::TooManyKeys => {
                vellum_kernel::ErrorKind::Conflict
            }
            E2eError::WeakKey => vellum_kernel::ErrorKind::InvalidInput,
        };
        Self::new(kind, err.to_string())
    }
}

/// Keys that clients must strip from every envelope in the space.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use = "clients must remove these recipients from existing envelopes"]
pub struct Removed {
    /// The space whose envelopes need re-wrapping.
    pub space: SpaceId,
    /// The removed device keys.
    pub keys: Vec<DeviceKey>,
}

/// The public keys of each E2E space's member devices.
///
/// This is the in-memory model; the pages service persists it in the
/// index and replays changes through the same methods.
#[derive(Debug, Clone, Default)]
pub struct KeyRegistry {
    spaces: BTreeMap<SpaceId, BTreeMap<UserId, Vec<DeviceKey>>>,
}

impl KeyRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a device key for `user` in `space`. The caller has already
    /// checked that `user` is a member and that the request comes from
    /// that user's signed-in device.
    ///
    /// # Errors
    ///
    /// [`E2eError::Duplicate`] or [`E2eError::TooManyKeys`].
    pub fn enroll(&mut self, space: SpaceId, user: UserId, key: DeviceKey) -> Result<(), E2eError> {
        let members = self.spaces.entry(space).or_default();
        if members.values().flatten().any(|k| *k == key) {
            return Err(E2eError::Duplicate);
        }
        if members.values().map(Vec::len).sum::<usize>() >= MAX_RECIPIENTS {
            return Err(E2eError::TooManyKeys);
        }
        members.entry(user).or_default().push(key);
        tracing::info!(
            space = %space,
            user = %user,
            monotonic_counter.vellum.e2e.keys_enrolled = 1_u64,
            "e2e device key enrolled"
        );
        Ok(())
    }

    /// Every device key in `space`: the recipients for a new document.
    #[must_use]
    pub fn recipients(&self, space: SpaceId) -> Vec<DeviceKey> {
        self.spaces
            .get(&space)
            .map(|m| m.values().flatten().copied().collect())
            .unwrap_or_default()
    }

    /// The device keys of one member.
    #[must_use]
    pub fn keys_of(&self, space: SpaceId, user: UserId) -> &[DeviceKey] {
        self.spaces
            .get(&space)
            .and_then(|m| m.get(&user))
            .map_or(&[], Vec::as_slice)
    }

    /// Removes one device key, for a lost or retired device.
    ///
    /// # Errors
    ///
    /// [`E2eError::NotEnrolled`] or [`E2eError::LastKey`].
    pub fn revoke_device(
        &mut self,
        space: SpaceId,
        user: UserId,
        key: DeviceKey,
    ) -> Result<Removed, E2eError> {
        let members = self.spaces.get_mut(&space).ok_or(E2eError::NotEnrolled)?;
        if members.values().map(Vec::len).sum::<usize>() <= 1 {
            return Err(E2eError::LastKey);
        }
        let keys = members.get_mut(&user).ok_or(E2eError::NotEnrolled)?;
        let before = keys.len();
        keys.retain(|k| *k != key);
        if keys.len() == before {
            return Err(E2eError::NotEnrolled);
        }
        if keys.is_empty() {
            let _gone = members.remove(&user);
        }
        Ok(Removed {
            space,
            keys: vec![key],
        })
    }

    /// Removes a member and all their device keys.
    ///
    /// # Errors
    ///
    /// [`E2eError::NotEnrolled`] or [`E2eError::LastKey`].
    pub fn remove_member(&mut self, space: SpaceId, user: UserId) -> Result<Removed, E2eError> {
        let members = self.spaces.get_mut(&space).ok_or(E2eError::NotEnrolled)?;
        if !members.contains_key(&user) {
            return Err(E2eError::NotEnrolled);
        }
        if members.len() == 1 {
            return Err(E2eError::LastKey);
        }
        let keys = members.remove(&user).unwrap_or_default();
        tracing::info!(space = %space, user = %user, keys = keys.len(), "e2e member removed");
        Ok(Removed { space, keys })
    }
}

/// A server-side feature that needs to read a document's plaintext.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ServerFeature {
    /// Full-text and semantic search on the server.
    Search,
    /// Server-side rendering, including diagrams and previews.
    Render,
    /// The built-in MCP server.
    Mcp,
    /// PDF, DOCX and site export.
    Export,
    /// Public or link-shared pages.
    PublicShare,
    /// Link previews and other fetches triggered by document content.
    Unfurl,
    /// Secret scanning on save.
    SecretScan,
    /// Git, S3 or Drive sync of readable files.
    PlaintextSync,
}

/// How a space stores its documents, as seen by server-side features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum SpaceMode {
    /// Readable by the server (encrypted at rest or plaintext).
    #[default]
    Server,
    /// End-to-end encrypted: the server holds only ciphertext.
    EndToEnd,
}

/// A feature refused for an E2E space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not available in an end-to-end encrypted space")]
pub struct NotInE2eSpace(pub ServerFeature);

impl From<NotInE2eSpace> for vellum_kernel::Error {
    fn from(err: NotInE2eSpace) -> Self {
        Self::new(vellum_kernel::ErrorKind::PermissionDenied, err.to_string())
    }
}

impl SpaceMode {
    /// Checks that `feature` may run on this space.
    ///
    /// # Errors
    ///
    /// [`NotInE2eSpace`] for every server feature in an E2E space.
    pub const fn allows(self, feature: ServerFeature) -> Result<(), NotInE2eSpace> {
        match self {
            Self::Server => Ok(()),
            Self::EndToEnd => Err(NotInE2eSpace(feature)),
        }
    }
}
