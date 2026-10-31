// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Errors and warnings for export and import.

use serde::Serialize;

/// Why an export failed. A block that cannot be shown never fails an
/// export: it becomes a readable fallback and an [`ExportWarning`].
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// The request was invalid (an unknown book node, an empty selection).
    #[error("invalid export request: {0}")]
    Invalid(String),
    /// The document is too large to export (too many images or pages).
    #[error("export limit exceeded: {0}")]
    TooLarge(String),
    /// The typesetter rejected the generated document. This is a bug in
    /// Vellum, never in the user's content.
    #[error("typesetting failed: {0}")]
    Typeset(String),
    /// Writing the output file failed.
    #[error("cannot write the output: {0}")]
    Write(String),
    /// An outside service (Gotenberg, Google Drive) failed or refused.
    #[error("{service} failed: {message}")]
    Remote {
        /// The service, such as `drive` or `gotenberg`.
        service: &'static str,
        /// What went wrong, without credentials.
        message: String,
        /// Whether retrying later may succeed.
        retryable: bool,
    },
    /// An outside service is not configured.
    #[error("{0} is not configured")]
    NotConfigured(&'static str),
}

impl ExportError {
    /// A stable `snake_case` code for metrics labels and API errors.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Invalid(_) => "invalid",
            Self::TooLarge(_) => "too_large",
            Self::Typeset(_) => "typeset",
            Self::Write(_) => "write",
            Self::Remote { .. } => "remote",
            Self::NotConfigured(_) => "not_configured",
        }
    }
}

/// Why an import failed. Content that cannot be mapped never fails an
/// import: it is kept as readable text and reported as a warning.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The archive or file is not what the importer expects.
    #[error("not a valid {format} export: {message}")]
    Format {
        /// The source format, such as `notion` or `docx`.
        format: &'static str,
        /// What is wrong with it.
        message: String,
    },
    /// The source is larger than the import limits allow.
    #[error("import limit exceeded: {0}")]
    TooLarge(String),
    /// Reading the source failed.
    #[error("cannot read the source: {0}")]
    Io(#[from] std::io::Error),
    /// An outside service (Google Drive) failed or refused.
    #[error("{service} failed: {message}")]
    Remote {
        /// The service, such as `drive`.
        service: &'static str,
        /// What went wrong, without credentials.
        message: String,
        /// Whether retrying later may succeed.
        retryable: bool,
    },
}

impl ImportError {
    /// A stable `snake_case` code for metrics labels and API errors.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Format { .. } => "format",
            Self::TooLarge(_) => "too_large",
            Self::Io(_) => "io",
            Self::Remote { .. } => "remote",
        }
    }

    pub(crate) fn format(
        format: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self::Format {
            format,
            message: message.into(),
        }
    }
}

impl From<zip::result::ZipError> for ImportError {
    fn from(error: zip::result::ZipError) -> Self {
        Self::format("zip", error.to_string())
    }
}

/// Something an export or import could not show exactly as written. The
/// output still contains a readable fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportWarning {
    /// Where: a block ID, a fence language, a file path or a page title.
    pub location: String,
    /// What happened, in a sentence for the person who asked.
    pub message: String,
}

impl ExportWarning {
    /// A warning at `location`.
    pub fn new(
        location: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            location: location.into(),
            message: message.into(),
        }
    }
}
