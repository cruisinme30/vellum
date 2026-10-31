// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The block-model generator.
//!
//! `schema/blocks.json` is the single source of the block model. This
//! command validates it and writes two views of it:
//!
//! - `web/src/editor/schema/blocks.gen.ts`, read by the Tiptap editor;
//! - `crates/vellum-contracts/src/blocks.rs`, read by the server.
//!
//! `cargo xtask schema --check` fails when either file is stale, so the
//! editor and the server can never drift apart.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::Deserialize;
use serde_json::Value;

const SOURCE: &str = "schema/blocks.json";
const TS_OUT: &str = "web/src/editor/schema/blocks.gen.ts";
const RS_OUT: &str = "crates/vellum-contracts/src/blocks.rs";

/// The whole schema file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schema {
    #[serde(rename = "$schema")]
    _uri: Option<String>,
    /// Bumped on any change that old documents must be migrated for.
    pub version: u32,
    /// Slash-menu sections, in display order.
    pub sections: Vec<String>,
    /// Node types.
    pub nodes: Vec<Node>,
    /// Mark types.
    pub marks: Vec<Mark>,
}

/// One node type.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools, reason = "mirrors the JSON flags")]
pub struct Node {
    /// The ProseMirror name, in camel case.
    pub name: String,
    /// One-line description, copied into both outputs.
    pub doc: String,
    /// Who implements it in the editor: `tiptap`, `generic` or `custom`.
    #[serde(rename = "impl")]
    pub implementation: String,
    /// Space-separated ProseMirror groups.
    #[serde(default)]
    pub group: String,
    /// ProseMirror content expression.
    #[serde(default)]
    pub content: String,
    /// Allowed marks; `None` means the ProseMirror default.
    pub marks: Option<String>,
    #[serde(default)]
    inline: bool,
    #[serde(default)]
    atom: bool,
    #[serde(default)]
    code: bool,
    #[serde(default)]
    defining: bool,
    #[serde(default)]
    isolating: bool,
    #[serde(default)]
    draggable: bool,
    /// Whether the node carries a stable `blockId` attribute.
    #[serde(default, rename = "blockId")]
    pub block_id: bool,
    /// The Vellum Markdown construct it maps to.
    pub markdown: Option<String>,
    /// Attributes by name.
    #[serde(default)]
    pub attrs: BTreeMap<String, Attr>,
    /// Slash-menu entries.
    #[serde(default)]
    pub slash: Vec<Slash>,
}

/// One mark type.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mark {
    /// The ProseMirror name.
    pub name: String,
    /// One-line description.
    pub doc: String,
    #[serde(rename = "impl")]
    implementation: String,
    /// The Vellum Markdown construct it maps to.
    pub markdown: String,
    excludes: Option<String>,
    #[serde(default)]
    attrs: BTreeMap<String, Attr>,
}

/// An attribute declaration.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attr {
    /// `string`, `integer`, `boolean` or `enum`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Allowed values of an enum.
    #[serde(default)]
    pub values: Vec<String>,
    /// The default value.
    pub default: Value,
    min: Option<i64>,
    max: Option<i64>,
    /// Editor-only state that is never saved.
    #[serde(default)]
    transient: bool,
}

/// A slash-menu entry.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slash {
    title: String,
    description: String,
    keywords: Vec<String>,
    section: String,
    icon: String,
    #[serde(default)]
    attrs: BTreeMap<String, Value>,
    template: Option<String>,
}

/// Runs `cargo xtask schema [--check]`.
pub fn run(args: &[String]) -> Result<(), String> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => return Err("usage: cargo xtask schema [--check]".to_owned()),
    };
    let root = workspace_root()?;
    let text = std::fs::read_to_string(root.join(SOURCE))
        .map_err(|e| format!("cannot read {SOURCE}: {e}"))?;
    let schema: Schema = serde_json::from_str(&text)
        .map_err(|e| format!("{SOURCE} is not valid: {e}"))?;
    validate(&schema)?;
    let outputs = [
        (TS_OUT, typescript(&schema)?),
        (RS_OUT, rustfmt(&rust(&schema)?)?),
    ];
    let mut stale = Vec::new();
    for (path, content) in outputs {
        let full = root.join(path);
        let current = std::fs::read_to_string(&full).unwrap_or_default();
        if current == content {
            continue;
        }
        if check {
            stale.push(path);
        } else {
            std::fs::write(&full, content)
                .map_err(|e| format!("cannot write {path}: {e}"))?;
        }
    }
    if stale.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "generated files are stale; run `cargo xtask schema`: {}",
            stale.join(", ")
        ))
    }
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").map_err(|_| {
        "run through `cargo xtask` so CARGO_MANIFEST_DIR is set".to_owned()
    })?;
    Path::new(&manifest)
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask has no parent directory".to_owned())
}

const IMPLS: [&str; 3] = ["tiptap", "generic", "custom"];
const ATTR_KINDS: [&str; 4] = ["string", "integer", "boolean", "enum"];

/// Checks every cross-reference in the schema.
pub fn validate(schema: &Schema) -> Result<(), String> {
    let mut errors = Vec::new();
    let mut names = BTreeSet::new();
    let mut groups = BTreeSet::new();
    for node in &schema.nodes {
        if !names.insert(node.name.as_str()) {
            errors.push(format!("node `{}` is declared twice", node.name));
        }
        groups.extend(node.group.split_whitespace());
    }
    for mark in &schema.marks {
        if !names.insert(mark.name.as_str()) {
            errors.push(format!("mark `{}` reuses a name", mark.name));
        }
        if !IMPLS.contains(&mark.implementation.as_str()) {
            errors.push(format!("mark `{}` has an unknown impl", mark.name));
        }
        check_attrs(&mark.name, &mark.attrs, &mut errors);
        if let Some(excludes) = &mark.excludes {
            for word in excludes.split_whitespace() {
                if word != "_" && !schema.marks.iter().any(|m| m.name == word) {
                    errors.push(format!(
                        "mark `{}` excludes unknown `{word}`",
                        mark.name
                    ));
                }
            }
        }
    }
    for name in &names {
        if !is_camel(name) {
            errors.push(format!("`{name}` is not camel case"));
        }
    }
    if !schema.nodes.iter().any(|n| n.name == "doc") {
        errors.push("the schema has no `doc` node".to_owned());
    }
    for node in &schema.nodes {
        check_node(schema, node, &names, &groups, &mut errors);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!("{SOURCE}:\n  {}", errors.join("\n  ")))
    }
}

fn check_node(
    schema: &Schema,
    node: &Node,
    names: &BTreeSet<&str>,
    groups: &BTreeSet<&str>,
    errors: &mut Vec<String>,
) {
    let name = &node.name;
    if !IMPLS.contains(&node.implementation.as_str()) {
        errors.push(format!("node `{name}` has an unknown impl"));
    }
    if node.markdown.is_none() && name != "doc" && name != "text" {
        errors.push(format!("node `{name}` has no markdown mapping"));
    }
    for word in content_names(&node.content) {
        if !names.contains(word) && !groups.contains(word) {
            errors
                .push(format!("node `{name}` content names unknown `{word}`"));
        }
    }
    if node.block_id && node.attrs.contains_key("blockId") {
        errors.push(format!("node `{name}` declares blockId twice"));
    }
    check_attrs(name, &node.attrs, errors);
    for entry in &node.slash {
        if !schema.sections.contains(&entry.section) {
            errors.push(format!(
                "`{}` uses unknown section `{}`",
                entry.title, entry.section
            ));
        }
        for key in entry.attrs.keys() {
            if !node.attrs.contains_key(key) {
                errors.push(format!(
                    "`{}` sets unknown attribute `{key}`",
                    entry.title
                ));
            }
        }
        if entry.keywords.is_empty()
            || entry.description.is_empty()
            || entry.icon.is_empty()
        {
            errors.push(format!(
                "`{}` needs keywords, a description and an icon",
                entry.title
            ));
        }
    }
}

fn check_attrs(
    owner: &str,
    attrs: &BTreeMap<String, Attr>,
    errors: &mut Vec<String>,
) {
    for (key, attr) in attrs {
        let fine = match attr.kind.as_str() {
            "string" => attr.default.is_string(),
            "boolean" => attr.default.is_boolean(),
            "integer" => attr.default.as_i64().is_some_and(|v| {
                attr.min.is_none_or(|min| v >= min)
                    && attr.max.is_none_or(|max| v <= max)
            }),
            "enum" => {
                !attr.values.is_empty()
                    && attr
                        .default
                        .as_str()
                        .is_some_and(|d| attr.values.iter().any(|v| v == d))
            }
            _ => false,
        };
        if !ATTR_KINDS.contains(&attr.kind.as_str()) || !fine {
            errors.push(format!("`{owner}.{key}` has a bad type or default"));
        }
    }
}

/// The node and group names a content expression mentions.
fn content_names(content: &str) -> impl Iterator<Item = &str> {
    content
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.starts_with(|c: char| c.is_ascii_alphabetic()))
}

fn is_camel(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_alphanumeric())
}

fn pascal(text: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if upper {
                out.push(c.to_ascii_uppercase());
            } else {
                out.push(c);
            }
            upper = false;
        } else {
            upper = true;
        }
    }
    out
}

fn json(value: &impl serde::Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

fn attrs_ts(attrs: &BTreeMap<String, Attr>) -> Result<String, String> {
    let mut out = String::from("{");
    for (key, attr) in attrs {
        let _ = write!(
            out,
            " {}: {{ type: {}, default: {}",
            json(key)?,
            json(&attr.kind)?,
            json(&attr.default)?
        );
        if !attr.values.is_empty() {
            let _ = write!(out, ", values: {}", json(&attr.values)?);
        }
        if attr.transient {
            out.push_str(", transient: true");
        }
        out.push_str(" },");
    }
    out.push_str(" }");
    Ok(out)
}

/// The licence header of both generated files. The tags are split so the
/// REUSE linter does not read them as this file's own header.
const HEADER: &str = concat!(
    "// SPDX-",
    "FileCopyrightText: 2026 The Vellum contributors\n",
    "// SPDX-",
    "License-Identifier: AGPL-3.0-only\n\n",
);

/// Renders the TypeScript view.
pub fn typescript(schema: &Schema) -> Result<String, String> {
    let mut out = String::from(HEADER);
    out.push_str(
        "// Generated by `cargo xtask schema` from schema/blocks.json. Do not \
         edit.\n\nimport type { MarkData, NodeData } from \"./types\";\n\n",
    );
    let _ =
        writeln!(out, "/** Bumped when saved documents must be migrated. */");
    let _ =
        writeln!(out, "export const SCHEMA_VERSION = {};\n", schema.version);
    let _ = writeln!(out, "/** Slash-menu sections, in display order. */");
    let _ = writeln!(
        out,
        "export const SECTIONS = {} as const;\n",
        json(&schema.sections)?
    );
    let _ = writeln!(out, "/** Every node type of the block model. */");
    out.push_str("export const NODES = [\n");
    for node in &schema.nodes {
        let flags = [
            ("inline", node.inline),
            ("atom", node.atom),
            ("code", node.code),
            ("defining", node.defining),
            ("isolating", node.isolating),
            ("draggable", node.draggable),
            ("blockId", node.block_id),
        ];
        let _ = write!(
            out,
            "  {{\n    name: {},\n    doc: {},\n    impl: {},\n    group: \
                {},\n    content: {},\n",
            json(&node.name)?,
            json(&node.doc)?,
            json(&node.implementation)?,
            json(&node.group)?,
            json(&node.content)?
        );
        if let Some(marks) = &node.marks {
            let _ = writeln!(out, "    marks: {},", json(marks)?);
        }
        for (flag, on) in flags {
            if on {
                let _ = writeln!(out, "    {flag}: true,");
            }
        }
        let _ = writeln!(
            out,
            "    markdown: {},",
            json(&node.markdown.clone().unwrap_or_default())?
        );
        let _ = writeln!(out, "    attrs: {},", attrs_ts(&node.attrs)?);
        out.push_str("    slash: [\n");
        for entry in &node.slash {
            let _ = write!(
                out,
                "      {{ title: {}, description: {}, keywords: {}, section: \
                    {}, icon: {}, attrs: {}",
                json(&entry.title)?,
                json(&entry.description)?,
                json(&entry.keywords)?,
                json(&entry.section)?,
                json(&entry.icon)?,
                json(&entry.attrs)?
            );
            if let Some(template) = &entry.template {
                let _ = write!(out, ", template: {}", json(template)?);
            }
            out.push_str(" },\n");
        }
        out.push_str("    ],\n  },\n");
    }
    out.push_str("] as const satisfies readonly NodeData[];\n\n");
    let _ = writeln!(out, "/** Every mark type of the block model. */");
    out.push_str("export const MARKS = [\n");
    for mark in &schema.marks {
        let _ = write!(
            out,
            "  {{ name: {}, doc: {}, impl: {}, markdown: {}, attrs: {}",
            json(&mark.name)?,
            json(&mark.doc)?,
            json(&mark.implementation)?,
            json(&mark.markdown)?,
            attrs_ts(&mark.attrs)?
        );
        if let Some(excludes) = &mark.excludes {
            let _ = write!(out, ", excludes: {}", json(excludes)?);
        }
        out.push_str(" },\n");
    }
    out.push_str("] as const satisfies readonly MarkData[];\n\n");
    out.push_str(
        "/** The name of a node type. */\n\
        export type NodeName = (typeof NODES)[number][\"name\"];\n\n",
    );
    out.push_str(
        "/** The name of a mark type. */\n\
        export type MarkName = (typeof MARKS)[number][\"name\"];\n",
    );
    Ok(out)
}

fn rust_str(text: &str) -> Result<String, String> {
    json(&text)
}

/// Renders the Rust view (before rustfmt).
pub fn rust(schema: &Schema) -> Result<String, String> {
    let mut out = String::from(HEADER);
    out.push_str(
        "// Generated by `cargo xtask schema` from schema/blocks.json. Do not \
         edit.\n\n//! The editor's block model, generated from \
         `schema/blocks.json`.\n//!\n\
         //! The editor stores documents as ProseMirror JSON and Yjs updates \
         whose\n//! node and mark names are listed here; `vellum-format` \
         converts them to\n//! and from Vellum Markdown.\n\n",
    );
    let _ = writeln!(out, "/// Bumped when saved documents must be migrated.");
    let _ =
        writeln!(out, "pub const SCHEMA_VERSION: u32 = {};\n", schema.version);
    rust_nodes(&mut out, schema)?;
    rust_marks(&mut out, schema)?;
    rust_attr_enums(&mut out, schema)?;
    Ok(out)
}

fn rust_nodes(out: &mut String, schema: &Schema) -> Result<(), String> {
    let nodes: Vec<(&str, &str)> = schema
        .nodes
        .iter()
        .map(|n| (n.name.as_str(), n.doc.as_str()))
        .collect();
    rust_enum(out, "NodeType", "A node type of the block model.", &nodes);
    let _ = writeln!(out, "impl NodeType {{");
    rust_all(out, &nodes);
    rust_names(out, &nodes)?;
    let block_ids: Vec<&str> = schema
        .nodes
        .iter()
        .filter(|n| n.block_id)
        .map(|n| n.name.as_str())
        .collect();
    rust_predicate(
        out,
        "has_block_id",
        "Whether the node carries a stable `blockId` attribute.",
        &block_ids,
    );
    let inline: Vec<&str> = schema
        .nodes
        .iter()
        .filter(|n| n.inline || n.name == "text")
        .map(|n| n.name.as_str())
        .collect();
    rust_predicate(out, "is_inline", "Whether the node is inline.", &inline);
    rust_lookup(
        out,
        "content",
        "The `ProseMirror` content expression; empty for leaves.",
        schema
            .nodes
            .iter()
            .map(|n| (n.name.as_str(), n.content.as_str())),
    )?;
    rust_lookup(
        out,
        "markdown",
        "The Vellum Markdown construct, such as `container:callout`; empty for \
            `doc` and `text`.",
        schema
            .nodes
            .iter()
            .map(|n| (n.name.as_str(), n.markdown.as_deref().unwrap_or(""))),
    )?;
    out.push_str("}\n\n");
    Ok(())
}

fn rust_marks(out: &mut String, schema: &Schema) -> Result<(), String> {
    let marks: Vec<(&str, &str)> = schema
        .marks
        .iter()
        .map(|m| (m.name.as_str(), m.doc.as_str()))
        .collect();
    rust_enum(out, "MarkType", "A mark type of the block model.", &marks);
    let _ = writeln!(out, "impl MarkType {{");
    rust_all(out, &marks);
    rust_names(out, &marks)?;
    rust_lookup(
        out,
        "markdown",
        "The Vellum Markdown construct.",
        schema
            .marks
            .iter()
            .map(|m| (m.name.as_str(), m.markdown.as_str())),
    )?;
    out.push_str("}\n\n");
    Ok(())
}

fn rust_attr_enums(out: &mut String, schema: &Schema) -> Result<(), String> {
    for node in &schema.nodes {
        for (key, attr) in &node.attrs {
            if attr.kind != "enum" {
                continue;
            }
            let ty = format!("{}{}", pascal(&node.name), pascal(key));
            let docs: Vec<String> =
                attr.values.iter().map(|v| format!("`{v}`")).collect();
            let values: Vec<(&str, &str)> = attr
                .values
                .iter()
                .zip(&docs)
                .map(|(v, d)| (v.as_str(), d.as_str()))
                .collect();
            let doc = format!("Values of `{}.{key}`.", node.name);
            rust_enum(out, &ty, &doc, &values);
            let _ = writeln!(out, "impl {ty} {{");
            rust_all(out, &values);
            rust_names(out, &values)?;
            let _ = writeln!(
                out,
                "    /// The default value.\n    pub const DEFAULT: Self = \
                    Self::{};",
                pascal(attr.default.as_str().unwrap_or_default())
            );
            out.push_str("}\n\n");
        }
    }
    Ok(())
}

fn rust_enum(out: &mut String, ty: &str, doc: &str, variants: &[(&str, &str)]) {
    let _ = writeln!(
        out,
        "/// {doc}\n#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n\
            pub enum {ty} {{"
    );
    for (name, vdoc) in variants {
        let _ = writeln!(
            out,
            "    /// {}\n    {},",
            vdoc.replace('\n', " "),
            pascal(name)
        );
    }
    out.push_str("}\n\n");
}

fn rust_all(out: &mut String, variants: &[(&str, &str)]) {
    let list: Vec<String> = variants
        .iter()
        .map(|(n, _)| format!("Self::{}", pascal(n)))
        .collect();
    let _ = writeln!(
        out,
        "    /// Every value, in schema order.\n    pub const ALL: [Self; {}] \
            = [{}];\n",
        variants.len(),
        list.join(", ")
    );
}

fn rust_names(
    out: &mut String,
    variants: &[(&str, &str)],
) -> Result<(), String> {
    out.push_str(
        "    /// The name used in documents.\n    #[must_use]\n    \
        pub const fn name(self) -> &'static str {\n        match self {\n",
    );
    for (name, _) in variants {
        let _ = writeln!(
            out,
            "            Self::{} => {},",
            pascal(name),
            rust_str(name)?
        );
    }
    out.push_str("        }\n    }\n\n");
    out.push_str(
        "    /// Looks a value up by its name.\n    #[must_use]\n    \
        pub fn from_name(name: &str) -> Option<Self> {\n        \
        Self::ALL.into_iter().find(|v| v.name() == name)\n    }\n\n",
    );
    Ok(())
}

fn rust_predicate(out: &mut String, func: &str, doc: &str, yes: &[&str]) {
    let arms: Vec<String> =
        yes.iter().map(|n| format!("Self::{}", pascal(n))).collect();
    let _ = writeln!(
        out,
        "    /// {doc}\n    #[must_use]\n    pub const fn {func}(self) -> bool \
            {{\n        matches!(self, {})\n    }}\n",
        arms.join(" | ")
    );
}

fn rust_lookup<'a>(
    out: &mut String,
    func: &str,
    doc: &str,
    rows: impl Iterator<Item = (&'a str, &'a str)>,
) -> Result<(), String> {
    let _ = writeln!(
        out,
        "    /// {doc}\n    #[must_use]\n    #[allow(clippy::match_same_arms, \
            reason = \"a generated lookup table\")]\n    pub const fn \
            {func}(self) -> &'static str {{\n        match self {{"
    );
    for (name, value) in rows {
        let _ = writeln!(
            out,
            "            Self::{} => {},",
            pascal(name),
            rust_str(value)?
        );
    }
    out.push_str("        }\n    }\n\n");
    Ok(())
}

fn rustfmt(source: &str) -> Result<String, String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout", "--quiet"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run rustfmt: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("rustfmt has no stdin")?
        .write_all(source.as_bytes())
        .map_err(|e| format!("cannot feed rustfmt: {e}"))?;
    let output = child
        .wait_with_output()
        .map_err(|e| format!("rustfmt failed: {e}"))?;
    if !output.status.success() {
        return Err("rustfmt rejected the generated Rust".to_owned());
    }
    String::from_utf8(output.stdout).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load() -> Schema {
        let root = workspace_root().unwrap_or_default();
        let text =
            std::fs::read_to_string(root.join(SOURCE)).unwrap_or_default();
        serde_json::from_str(&text).unwrap_or_else(|_| Schema {
            _uri: None,
            version: 0,
            sections: Vec::new(),
            nodes: Vec::new(),
            marks: Vec::new(),
        })
    }

    #[test]
    fn the_checked_in_schema_is_valid() {
        let schema = load();
        assert!(schema.version >= 1);
        assert_eq!(validate(&schema), Ok(()));
    }

    #[test]
    fn validation_reports_unknown_content_and_bad_defaults() {
        let mut schema = load();
        if let Some(node) =
            schema.nodes.iter_mut().find(|n| n.name == "callout")
        {
            node.content = "nothing+".to_owned();
            if let Some(kind) = node.attrs.get_mut("kind") {
                kind.default = Value::from("purple");
            }
        }
        let message = validate(&schema).err().unwrap_or_default();
        assert!(message.contains("unknown `nothing`"), "{message}");
        assert!(
            message.contains("`callout.kind` has a bad type or default"),
            "{message}"
        );
    }

    #[test]
    fn names_convert_to_pascal_case() {
        assert_eq!(pascal("risk-matrix"), "RiskMatrix");
        assert_eq!(pascal("dataFence"), "DataFence");
        assert!(is_camel("taskItem"));
        assert!(!is_camel("TaskItem"));
        assert_eq!(
            content_names("(tableCell | tableHeader)*").collect::<Vec<_>>(),
            ["tableCell", "tableHeader"]
        );
    }

    #[test]
    fn outputs_name_every_node() {
        let schema = load();
        let ts = typescript(&schema).unwrap_or_default();
        let rs = rust(&schema).unwrap_or_default();
        for node in &schema.nodes {
            assert!(ts.contains(&format!("name: \"{}\"", node.name)));
            assert!(rs.contains(&format!(
                "Self::{} => \"{}\"",
                pascal(&node.name),
                node.name
            )));
        }
    }
}
