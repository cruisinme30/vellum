// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The error type for every Work operation.

use crate::id::IssueId;
use crate::team::TeamKey;

/// Everything that can go wrong in the Work engine.
///
/// Each variant names the input that was refused, so the API layer can map
/// it to a field-level validation message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum WorkError {
    /// An identifier, key or other value is malformed.
    #[error("invalid {what}: {value:?} ({reason})")]
    Invalid {
        /// What kind of value was refused, such as "team key".
        what: &'static str,
        /// The refused value.
        value: String,
        /// Why it was refused.
        reason: &'static str,
    },
    /// A team's configuration breaks a rule.
    #[error("team {team}: {reason}")]
    TeamConfig {
        /// The team whose configuration is invalid.
        team: TeamKey,
        /// The rule that was broken.
        reason: String,
    },
    /// No team has this key.
    #[error("unknown team {0}")]
    UnknownTeam(TeamKey),
    /// A team with this key or ID already exists.
    #[error("team {0} already exists")]
    DuplicateTeam(TeamKey),
    /// No issue has this ID or identifier.
    #[error("unknown issue {0}")]
    UnknownIssue(String),
    /// An issue with this ID already exists.
    #[error("issue {0} already exists")]
    DuplicateIssue(IssueId),
    /// Two issues claim the same identifier.
    #[error("identifier {0} is already used")]
    DuplicateKey(String),
    /// The status is not part of the team's workflow.
    #[error("team {team} has no status {status:?}")]
    UnknownStatus {
        /// The team whose workflow was searched.
        team: TeamKey,
        /// The status key that was not found.
        status: String,
    },
    /// The estimate is not on the team's scale.
    #[error("estimate {value} is not on the {scale} scale")]
    InvalidEstimate {
        /// The refused estimate, as written.
        value: String,
        /// The team's scale.
        scale: &'static str,
    },
    /// The operation needs the issue to be in a different state.
    #[error("issue {issue}: {reason}")]
    InvalidState {
        /// The issue the operation was attempted on.
        issue: String,
        /// Why the operation is not allowed now.
        reason: &'static str,
    },
    /// A relation would point at the issue itself or form a parent cycle.
    #[error("issue {issue}: {reason}")]
    InvalidRelation {
        /// The issue the relation was added to.
        issue: String,
        /// Why the relation was refused.
        reason: &'static str,
    },
    /// A Work file could not be parsed.
    #[error("{path}: {reason}")]
    File {
        /// The file's path relative to the workspace root.
        path: String,
        /// What is wrong with it.
        reason: String,
    },
}

/// The result type of the Work engine.
pub type Result<T, E = WorkError> = std::result::Result<T, E>;

impl WorkError {
    /// The kernel error kind this maps to at service boundaries.
    #[must_use]
    pub const fn kind(&self) -> vellum_kernel::ErrorKind {
        use vellum_kernel::ErrorKind as K;
        match self {
            Self::UnknownTeam(_) | Self::UnknownIssue(_) => K::NotFound,
            Self::DuplicateTeam(_)
            | Self::DuplicateIssue(_)
            | Self::DuplicateKey(_)
            | Self::InvalidState { .. } => K::Conflict,
            Self::File { .. } => K::Internal,
            Self::Invalid { .. }
            | Self::TeamConfig { .. }
            | Self::UnknownStatus { .. }
            | Self::InvalidEstimate { .. }
            | Self::InvalidRelation { .. } => K::InvalidInput,
        }
    }
}

impl From<WorkError> for vellum_kernel::Error {
    fn from(e: WorkError) -> Self {
        Self::new(e.kind(), e.to_string()).with_source(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_kernel_kinds() {
        let e: vellum_kernel::Error = WorkError::UnknownIssue("ENG-9".into()).into();
        assert_eq!(e.kind(), vellum_kernel::ErrorKind::NotFound);
        assert_eq!(e.message(), "unknown issue ENG-9");
        let dup = WorkError::DuplicateKey("ENG-1".into());
        assert_eq!(dup.kind(), vellum_kernel::ErrorKind::Conflict);
    }
}
