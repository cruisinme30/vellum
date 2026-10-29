// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The YAML files the drive owns: folder metadata, shortcuts, trash
//! manifests and version sidecars. Each carries what the index needs, so
//! `vellum reindex` can rebuild the drive from the files alone.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use vellum_kernel::{Actor, ObjectId, SpaceId, Timestamp, UserId};
use vellum_store::BlobPath;

use crate::error::{DriveError, DriveResult};
use crate::model::{NodeKind, SpaceKind, SpaceSettings};

/// `.vellum-folder.yaml`: a folder's identity, or a space's at its root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderFile {
    /// The folder's (or space's) ID.
    pub id: ObjectId,
    /// Optional icon (emoji or icon name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Present only at a space root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<SpaceFile>,
}

/// The space section of a root `.vellum-folder.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpaceFile {
    /// Shared drive or My Drive.
    pub kind: SpaceKind,
    /// Display name.
    pub name: String,
    /// The person whose My Drive this is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<UserId>,
    /// Quota, trash and retention settings.
    #[serde(default)]
    pub settings: SpaceSettings,
}

/// `Name.shortcut.yaml`: a link object that points at a node by ID and
/// never grants access to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShortcutFile {
    /// The shortcut's own ID.
    pub id: ObjectId,
    /// The node it points at.
    pub target: ObjectId,
    /// Display title.
    pub title: String,
}

/// One node held by a trash entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashMember {
    /// The node.
    pub id: ObjectId,
    /// Its kind.
    pub kind: NodeKind,
    /// Its title.
    pub title: String,
    /// Where it lived.
    #[serde(with = "blob_path")]
    pub original_path: BlobPath,
    /// Its parent folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_parent: Option<ObjectId>,
    /// Who created it, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<UserId>,
}

/// `.vellum/trash/<id>/manifest.yaml`: one explicitly trashed node and
/// everything that went with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrashManifest {
    /// The space the entry belongs to.
    pub space: SpaceId,
    /// When it was trashed.
    pub trashed_at: Timestamp,
    /// Who trashed it.
    pub trashed_by: Actor,
    /// The explicitly trashed node first, then the nodes beneath it.
    pub members: Vec<TrashMember>,
}

/// `.vellum/versions/<doc>/<version>.yaml`: who wrote a version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionFile {
    /// The page's space.
    pub space: SpaceId,
    /// Who wrote it.
    pub author: Actor,
}

/// Serde support for [`BlobPath`] as a plain string.
pub mod blob_path {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};
    use vellum_store::BlobPath;

    /// Writes the path as a string.
    ///
    /// # Errors
    /// Only the serializer's own errors.
    pub fn serialize<S: Serializer>(path: &BlobPath, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(path.as_str())
    }

    /// Reads and validates a path.
    ///
    /// # Errors
    /// Fails for text that is not a valid [`BlobPath`].
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BlobPath, D::Error> {
        let text = String::deserialize(deserializer)?;
        BlobPath::new(text).map_err(D::Error::custom)
    }
}

/// Serialises a drive file.
///
/// # Errors
/// Fails only if the value cannot be represented in YAML.
pub fn to_yaml<T: Serialize>(value: &T) -> DriveResult<String> {
    serde_saphyr::to_string(value).map_err(|e| DriveError::Malformed {
        path: String::new(),
        reason: e.to_string(),
    })
}

/// Parses a drive file read from `path`.
///
/// # Errors
/// Returns [`DriveError::Malformed`] when the bytes are not the expected YAML.
pub fn from_yaml<T: DeserializeOwned>(path: &BlobPath, bytes: &[u8]) -> DriveResult<T> {
    serde_saphyr::from_slice(bytes).map_err(|e| DriveError::Malformed {
        path: path.to_string(),
        reason: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_files_round_trip() -> DriveResult<()> {
        let file = FolderFile {
            id: ObjectId::generate(),
            icon: Some("📁".into()),
            space: Some(SpaceFile {
                kind: SpaceKind::SharedDrive,
                name: "Engineering".into(),
                owner: None,
                settings: SpaceSettings {
                    quota_bytes: Some(1 << 30),
                    ..SpaceSettings::default()
                },
            }),
        };
        let text = to_yaml(&file)?;
        let path = BlobPath::new("drives/shared/eng/.vellum-folder.yaml")?;
        assert_eq!(from_yaml::<FolderFile>(&path, text.as_bytes())?, file);
        Ok(())
    }

    #[test]
    fn bad_yaml_is_malformed() -> DriveResult<()> {
        let path = BlobPath::new("x.shortcut.yaml")?;
        let err = from_yaml::<ShortcutFile>(&path, b"id: [").err();
        assert!(matches!(err, Some(DriveError::Malformed { .. })));
        Ok(())
    }
}
