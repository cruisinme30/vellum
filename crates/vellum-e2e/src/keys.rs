// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A space's key list, as stored in the blob store (D182).

use serde::{Deserialize, Serialize};
use vellum_kernel::{Error, ErrorKind, SpaceId, Timestamp, UserId};
use vellum_security::e2e::{DeviceKey, KEY_LEN, KeyRegistry};
use vellum_store::BlobPath;
use vellum_store::layout::SYSTEM_DIR;

use crate::api::DeviceKeyInfo;

/// The folder under `.vellum/` that holds every space's key list.
pub const KEYS_DIR: &str = "e2e";

/// Format version written in each file.
const FORMAT: u32 = 1;

/// Where `space`'s key list lives: `.vellum/e2e/<space id>.yaml`. Files
/// under `.vellum/` are never indexed, listed by the drive or synced as
/// content.
///
/// # Errors
///
/// Never in practice: a space ID is always a valid file name.
pub fn keys_path(space: SpaceId) -> Result<BlobPath, Error> {
    BlobPath::new(format!("{SYSTEM_DIR}/{KEYS_DIR}/{space}.yaml"))
        .map_err(|e| Error::new(ErrorKind::Internal, "bad key list path").with_source(e))
}

/// The file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KeyFile {
    pub vellum: u32,
    #[serde(default)]
    pub keys: Vec<Entry>,
}

/// One enrolled device in the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub user: UserId,
    pub key: String,
    pub label: String,
    pub added: Timestamp,
}

impl Entry {
    pub fn device_key(&self) -> Result<DeviceKey, Error> {
        parse_key(&self.key)
            .map_err(|_| Error::new(ErrorKind::Internal, "the space's key list holds a bad key"))
    }

    pub fn info(&self, caller: UserId) -> Result<DeviceKeyInfo, Error> {
        let key = self.device_key()?;
        Ok(DeviceKeyInfo {
            user_id: self.user,
            public_key: self.key.clone(),
            fingerprint: hex::encode(key.fingerprint()),
            label: self.label.clone(),
            added_at: self.added,
            mine: self.user == caller,
        })
    }
}

impl KeyFile {
    pub const fn empty() -> Self {
        Self {
            vellum: FORMAT,
            keys: Vec::new(),
        }
    }

    pub fn parse(text: &str) -> Result<Self, Error> {
        let file: Self = serde_saphyr::from_str(text).map_err(|e| {
            Error::new(ErrorKind::Internal, "the space's key list cannot be read").with_source(e)
        })?;
        if file.vellum != FORMAT {
            return Err(Error::new(
                ErrorKind::Internal,
                "the space's key list has an unknown format",
            ));
        }
        Ok(file)
    }

    pub fn render(&self) -> Result<String, Error> {
        let body = serde_saphyr::to_string(self).map_err(|e| {
            Error::new(ErrorKind::Internal, "cannot write the key list").with_source(e)
        })?;
        Ok(format!(
            "# Public keys of the devices this end-to-end encrypted space is sealed to.\n\
             # Managed by Vellum (ADR 0043); edit through `vellum private` or the API.\n{body}"
        ))
    }

    /// The registry the file describes, which checks every change.
    pub fn registry(&self, space: SpaceId) -> Result<KeyRegistry, Error> {
        let mut registry = KeyRegistry::new();
        for entry in &self.keys {
            registry.enroll(space, entry.user, entry.device_key()?)?;
        }
        Ok(registry)
    }

    /// Fingerprints of every key, sorted.
    pub fn fingerprints(&self) -> Result<Vec<[u8; 16]>, Error> {
        let mut out = self
            .keys
            .iter()
            .map(|e| e.device_key().map(DeviceKey::fingerprint))
            .collect::<Result<Vec<_>, _>>()?;
        out.sort_unstable();
        Ok(out)
    }
}

/// Parses a public key sent by a client: 64 hex digits, not a weak point.
pub fn parse_key(text: &str) -> Result<DeviceKey, Error> {
    let invalid = || {
        Error::new(
            ErrorKind::InvalidInput,
            "public_key must be 64 hex digits, as `vellum private key` prints it",
        )
    };
    let bytes = hex::decode(text.trim()).map_err(|_| invalid())?;
    let bytes: [u8; KEY_LEN] = bytes.try_into().map_err(|_| invalid())?;
    Ok(DeviceKey::new(bytes)?)
}
