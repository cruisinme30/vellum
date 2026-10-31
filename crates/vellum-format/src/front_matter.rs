// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! YAML front matter between `---` lines at the top of a file.
//!
//! Known page fields come first in a fixed order (`id`, `title`, `icon`,
//! `cover`, `status`, `owner`, `tags`, `verify_every`, `vellum`); other keys
//! (issue, task and row fields) follow in the order they were written.
//!
//! YAML is read and written with `serde-saphyr`, a parser in safe Rust,
//! into JSON-shaped values: front matter keys are strings, and values are
//! strings, numbers, booleans, lists and mappings.

use serde::Serialize;
pub use serde_json::{Map, Value};
use vellum_kernel::ObjectId;

use crate::diagnostic::{Diagnostic, DiagnosticCode};

/// Page fields, in canonical order.
pub const KNOWN_KEYS: [&str; 9] = [
    "id",
    "title",
    "icon",
    "cover",
    "status",
    "owner",
    "tags",
    "verify_every",
    "vellum",
];

/// The dialect version this crate reads and writes (`vellum: 1`).
pub const DIALECT_VERSION: u64 = 1;

/// Parsed front matter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FrontMatter {
    /// A YAML mapping, keys in canonical order.
    Fields(Map<String, Value>),
    /// YAML that could not be read as a mapping, kept verbatim (each line
    /// ending in `\n`) so the file survives a round trip.
    Raw(String),
}

/// A page's review status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// `draft`
    Draft,
    /// `in-review`
    InReview,
    /// `verified`
    Verified,
    /// `outdated`
    Outdated,
}

impl Status {
    /// Reads a status word.
    #[must_use]
    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "draft" => Some(Self::Draft),
            "in-review" => Some(Self::InReview),
            "verified" => Some(Self::Verified),
            "outdated" => Some(Self::Outdated),
            _ => None,
        }
    }
}

impl FrontMatter {
    /// The value of `key`, if the front matter is a mapping and has it.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Fields(map) => map.get(key),
            Self::Raw(_) => None,
        }
    }

    /// The object's identity, when `id` holds a valid ULID.
    #[must_use]
    pub fn id(&self) -> Option<ObjectId> {
        self.get("id")?.as_str()?.parse().ok()
    }

    /// The page title.
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.get("title")?.as_str()
    }

    /// The review status, when valid.
    #[must_use]
    pub fn status(&self) -> Option<Status> {
        Status::from_word(self.get("status")?.as_str()?)
    }

    /// The `tags` list; non-string entries are skipped.
    #[must_use]
    pub fn tags(&self) -> Vec<&str> {
        self.get("tags")
            .and_then(Value::as_array)
            .map(|tags| tags.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }
}

/// Splits front matter off the start of `src`.
///
/// Returns the YAML text, the rest of the file, and how many lines the front
/// matter took. Returns `None` when `src` does not start with a `---` line
/// that has a matching `---` or `...` line after it.
#[must_use]
pub fn split(src: &str) -> Option<(&str, &str, usize)> {
    let rest = src.strip_prefix("---\n")?;
    let mut offset = 0;
    for (index, line) in rest.split_inclusive('\n').enumerate() {
        let bare = line.strip_suffix('\n').unwrap_or(line);
        if bare == "---" || bare == "..." {
            let yaml = rest.get(..offset)?;
            let body = rest.get(offset + line.len()..)?;
            return Some((yaml, body, index + 2));
        }
        offset += line.len();
    }
    None
}

/// Reads front matter YAML. Never fails: bad YAML is kept verbatim and
/// reported in `diagnostics`.
pub fn parse(yaml: &str, diagnostics: &mut Vec<Diagnostic>) -> FrontMatter {
    if yaml.trim().is_empty() {
        return FrontMatter::Fields(Map::new());
    }
    let value = match serde_saphyr::from_str_with_options::<Value>(
        yaml,
        yaml_options(),
    ) {
        Ok(value) => value,
        Err(error) => {
            // The YAML starts on line 2 of the file, after the `---` line.
            let line = error
                .location()
                .and_then(|l| usize::try_from(l.line()).ok())
                .map_or(2, |l| l + 1);
            let message = error.without_snippet().to_string();
            let message = message.lines().next().unwrap_or_default();
            diagnostics.push(Diagnostic::new(
                line,
                DiagnosticCode::FrontMatterYaml,
                format!("front matter is not valid YAML: {message}"),
            ));
            return FrontMatter::Raw(yaml.to_owned());
        }
    };
    let map = match value {
        Value::Object(map) => map,
        Value::Null => Map::new(),
        _ => {
            diagnostics.push(Diagnostic::new(
                2,
                DiagnosticCode::FrontMatterShape,
                "front matter must be a YAML mapping of keys to values",
            ));
            return FrontMatter::Raw(yaml.to_owned());
        }
    };
    let map = canonical_order(map);
    check_fields(&map, diagnostics);
    FrontMatter::Fields(map)
}

/// Limits for front matter, far tighter than the library defaults: front
/// matter is a short list of properties, and the default nesting depth of
/// 64 is enough to overflow a 1 `MiB` stack in a debug build (D120).
fn yaml_options() -> serde_saphyr::Options {
    serde_saphyr::options! {
        budget: serde_saphyr::budget! {
            max_depth: 16,
            max_nodes: 20_000,
            max_aliases: 1_000,
            max_anchors: 1_000,
            max_events: 100_000,
            max_total_scalar_bytes: 4 * 1024 * 1024,
        },
    }
}

fn canonical_order(mut map: Map<String, Value>) -> Map<String, Value> {
    let mut ordered = Map::with_capacity(map.len());
    for key in KNOWN_KEYS {
        if let Some(value) = map.shift_remove(key) {
            let _ = ordered.insert(key.to_owned(), value);
        }
    }
    ordered.extend(map);
    ordered
}

fn check_fields(map: &Map<String, Value>, diagnostics: &mut Vec<Diagnostic>) {
    let mut field = |message: String| {
        diagnostics.push(Diagnostic::new(
            0,
            DiagnosticCode::FrontMatterField,
            message,
        ));
    };
    if let Some(id) = map.get("id")
        && id
            .as_str()
            .and_then(|s| s.parse::<ObjectId>().ok())
            .is_none()
    {
        field(format!(
            "`id` must be a 26-character ULID, found {}",
            show(id)
        ));
    }
    if let Some(status) = map.get("status")
        && status.as_str().and_then(Status::from_word).is_none()
    {
        field(format!(
            "`status` must be draft, in-review, verified or outdated, found {}",
            show(status)
        ));
    }
    if let Some(version) = map.get("vellum")
        && version.as_u64() != Some(DIALECT_VERSION)
    {
        field(format!(
            "`vellum` must be {DIALECT_VERSION}, found {}",
            show(version)
        ));
    }
    if let Some(tags) = map.get("tags")
        && !tags
            .as_array()
            .is_some_and(|t| t.iter().all(Value::is_string))
    {
        field(format!(
            "`tags` must be a list of words, found {}",
            show(tags)
        ));
    }
}

fn show(value: &Value) -> String {
    serde_saphyr::to_string(value)
        .map(|s| s.trim_end().to_owned())
        .unwrap_or_default()
}

/// Writes front matter, including both `---` lines, in canonical form.
pub fn write(front_matter: &FrontMatter, out: &mut String) {
    out.push_str("---\n");
    match front_matter {
        FrontMatter::Fields(map) if map.is_empty() => {}
        FrontMatter::Fields(map) => match serde_saphyr::to_string(map) {
            Ok(yaml) => out.push_str(&yaml),
            // A parsed mapping always serializes; keep the file valid anyway.
            Err(error) => {
                tracing::error!(%error, "front matter could not be written");
            }
        },
        FrontMatter::Raw(raw) => {
            out.push_str(raw);
            if !raw.is_empty() && !raw.ends_with('\n') {
                out.push('\n');
            }
        }
    }
    out.push_str("---\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "---\nvellum: 1\ntags: [launch, q4]\n\
                        title: Northstar launch plan\n\
                        id: 01JB7Z3K8Q4W5N6P7R8S9T0V1X\nreviewer: priya\n\
                        status: in-review\n---\n# Body\n";

    #[test]
    fn splits_and_orders_known_keys_first() {
        let (yaml, body, lines) = split(PAGE).unwrap();
        assert_eq!(body, "# Body\n");
        assert_eq!(lines, 8);
        let mut diagnostics = Vec::new();
        let fm = parse(yaml, &mut diagnostics);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(fm.title(), Some("Northstar launch plan"));
        assert_eq!(fm.status(), Some(Status::InReview));
        assert_eq!(fm.tags(), ["launch", "q4"]);
        assert!(fm.id().is_some());
        let mut out = String::new();
        write(&fm, &mut out);
        assert_eq!(
            out,
            "---\nid: 01JB7Z3K8Q4W5N6P7R8S9T0V1X\n\
             title: Northstar launch plan\nstatus: in-review\ntags:\n- launch\n\
             - q4\nvellum: 1\nreviewer: priya\n---\n"
        );
    }

    #[test]
    fn needs_a_closing_line() {
        assert!(split("---\ntitle: x\n").is_none());
        assert!(split("--- \ntitle: x\n---\n").is_none());
        assert_eq!(split("---\n...\nrest").map(|s| s.1), Some("rest"));
    }

    #[test]
    fn keeps_bad_yaml_verbatim() {
        let mut diagnostics = Vec::new();
        let fm = parse("title: [unclosed\n", &mut diagnostics);
        assert_eq!(fm, FrontMatter::Raw("title: [unclosed\n".into()));
        assert_eq!(diagnostics[0].code, DiagnosticCode::FrontMatterYaml);
        let fm = parse("- a list\n", &mut diagnostics);
        assert!(matches!(fm, FrontMatter::Raw(_)));
        assert_eq!(diagnostics[1].code, DiagnosticCode::FrontMatterShape);
    }

    #[test]
    fn reports_invalid_known_fields() {
        let mut diagnostics = Vec::new();
        let _ = parse(
            "id: nope\nstatus: done\nvellum: 2\ntags: 3\n",
            &mut diagnostics,
        );
        assert_eq!(diagnostics.len(), 4);
        assert!(
            diagnostics
                .iter()
                .all(|d| d.code == DiagnosticCode::FrontMatterField)
        );
    }

    #[test]
    fn empty_front_matter_round_trips() {
        let mut diagnostics = Vec::new();
        let fm = parse("", &mut diagnostics);
        let mut out = String::new();
        write(&fm, &mut out);
        assert_eq!(out, "---\n---\n");
    }
}
