// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Errors and load diagnostics.

use serde::Serialize;
use vellum_kernel::{CollectionId, DocId, ErrorKind};

use crate::formula::FormulaError;

/// Why a `schema.yaml` (or a schema change) was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SchemaError {
    /// The YAML could not be read.
    #[error("schema.yaml is not valid: {0}")]
    Yaml(String),
    /// The schema has no `id`.
    #[error("schema.yaml needs an id")]
    MissingId,
    /// A property key is not `[a-z][a-z0-9_]*` of at most 32 characters.
    #[error(
        "property key {0:?} is invalid: use a-z, 0-9 and _, starting with a \
            letter"
    )]
    InvalidKey(String),
    /// A property key is reserved for page front matter.
    #[error("property key {0:?} is reserved for page front matter")]
    ReservedKey(String),
    /// Two properties share a key.
    #[error("duplicate property key {0:?}")]
    DuplicateKey(String),
    /// Two properties share a name (case-insensitively).
    #[error("duplicate property name {0:?}")]
    DuplicateName(String),
    /// A property has a blank name.
    #[error("property {0:?} needs a name")]
    EmptyName(String),
    /// More properties than [`crate::schema::MAX_PROPERTIES`].
    #[error("a collection may have at most 256 properties, got {0}")]
    TooManyProperties(usize),
    /// The collection does not have exactly one title property.
    #[error("a collection needs exactly one title property, found {0}")]
    TitleCount(usize),
    /// A property's `type` is not a known type.
    #[error("property {key:?} has unknown type {kind:?}")]
    UnknownType {
        /// The property key.
        key: String,
        /// The type that was given.
        kind: String,
    },
    /// A required type-specific field is missing.
    #[error("property {key:?} needs a {field:?} field")]
    MissingField {
        /// The property key.
        key: String,
        /// The missing field.
        field: &'static str,
    },
    /// A formula does not compile.
    #[error("formula {key:?}: {error}")]
    Formula {
        /// The property key.
        key: String,
        /// The compile error, with its position.
        error: FormulaError,
    },
    /// Formulas refer to each other in a loop.
    #[error("formulas are circular: {}", .0.join(" → "))]
    FormulaCycle(Vec<String>),
    /// A rollup's `via` is not a relation property of this collection.
    #[error("rollup {key:?}: {via:?} is not a relation property here")]
    RollupVia {
        /// The rollup property key.
        key: String,
        /// The `via` that was given.
        via: String,
    },
    /// A rollup's `prop` does not exist on the related collection.
    #[error("rollup {key:?}: the related collection has no property {prop:?}")]
    RollupProp {
        /// The rollup property key.
        key: String,
        /// The `prop` that was given.
        prop: String,
    },
    /// A relation's `back` does not pair up with a relation pointing back.
    #[error(
        "relation {key:?}: {back:?} must be a relation in the other collection \
            pointing back with back: {key}"
    )]
    RelationBack {
        /// The relation property key.
        key: String,
        /// The `back` that was given.
        back: String,
    },
    /// A view refers to something that does not exist.
    #[error("view {view:?}: {message}")]
    View {
        /// The view name.
        view: String,
        /// What is wrong.
        message: String,
    },
}

/// Why an edit to a collection was rejected.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum EditError {
    /// The collection is not loaded.
    #[error("collection {0} not found")]
    UnknownCollection(CollectionId),
    /// The row does not exist in the collection.
    #[error("row {0} not found")]
    UnknownRow(DocId),
    /// A row with this ID already exists.
    #[error("row {0} already exists")]
    DuplicateRow(DocId),
    /// The property does not exist.
    #[error("no property {0:?}")]
    UnknownProperty(String),
    /// The property is computed and cannot be written.
    #[error("property {0:?} is computed and cannot be set")]
    Computed(String),
    /// The value does not fit the property type.
    #[error("property {key:?} expects {expected}, got {got}")]
    WrongType {
        /// The property key.
        key: String,
        /// The expected shape.
        expected: &'static str,
        /// The type that was given.
        got: &'static str,
    },
    /// The schema change is invalid.
    #[error(transparent)]
    Schema(#[from] SchemaError),
}

impl From<SchemaError> for vellum_kernel::Error {
    fn from(error: SchemaError) -> Self {
        Self::new(ErrorKind::InvalidInput, error.to_string()).with_source(error)
    }
}

impl From<EditError> for vellum_kernel::Error {
    fn from(error: EditError) -> Self {
        let kind = match &error {
            EditError::UnknownCollection(_) | EditError::UnknownRow(_) => {
                ErrorKind::NotFound
            }
            EditError::DuplicateRow(_) => ErrorKind::Conflict,
            _ => ErrorKind::InvalidInput,
        };
        Self::new(kind, error.to_string()).with_source(error)
    }
}

/// How serious a diagnostic is.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Data was kept but may not mean what the author intended.
    Warning,
    /// Part of the input could not be used; it was preserved verbatim.
    Error,
}

/// A problem found while loading files or importing CSV.
///
/// Loading never fails on a bad row: the row still loads, the offending
/// value is preserved verbatim, and a diagnostic says what happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct Diagnostic {
    /// How serious it is.
    pub severity: Severity,
    /// The file or CSV line it refers to.
    pub location: String,
    /// What happened, for people.
    pub message: String,
}

impl Diagnostic {
    pub(crate) fn warning(
        location: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Warning,
            location: location.into(),
            message: message.into(),
        }
    }

    pub(crate) fn error(
        location: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            severity: Severity::Error,
            location: location.into(),
            message: message.into(),
        }
    }
}
