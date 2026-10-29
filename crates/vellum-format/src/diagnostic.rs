// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Problems found while parsing. Parsing never fails: a problem leaves a
//! readable fallback in the AST and a diagnostic here.

use serde::Serialize;

/// What kind of problem a [`Diagnostic`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    /// Front matter is not valid YAML; it is kept verbatim.
    FrontMatterYaml,
    /// Front matter is valid YAML but not a mapping; it is kept verbatim.
    FrontMatterShape,
    /// A known front-matter field has an invalid value (bad `id`, unknown
    /// `status`, unsupported `vellum` version).
    FrontMatterField,
    /// A `:::` container has no closing line; it runs to the end of its parent.
    UnclosedContainer,
    /// A `:::` container name is not part of dialect v1; its body is shown
    /// as ordinary blocks.
    UnknownContainer,
    /// Raw HTML was found; it is kept as literal text.
    RawHtml,
}

/// A problem found while parsing, with the 1-based source line when known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    /// 1-based line in the source file; 0 when unknown.
    pub line: usize,
    /// The kind of problem.
    pub code: DiagnosticCode,
    /// A human-readable explanation.
    pub message: String,
}

impl Diagnostic {
    /// Creates a diagnostic.
    pub fn new(line: usize, code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            line,
            code,
            message: message.into(),
        }
    }
}
