// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The production codec against the format corpus: every file survives
//! Markdown → tree → Yjs → tree → Markdown, and comes back as its
//! canonical form with the same AST.

#![allow(missing_docs, clippy::unwrap_used, clippy::indexing_slicing)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use proptest::prelude::*;
use vellum_collab::markdown::{from_document, to_document};
use vellum_collab::{FormatCodec, MarkdownCodec as _, Node, tree};
use vellum_contracts::blocks::{MarkType, NodeType};

/// Attribute names each node may carry, read from `schema/blocks.json`.
fn schema_attrs() -> HashMap<String, HashSet<String>> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/blocks.json");
    let schema: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    schema["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| {
            let mut attrs: HashSet<String> = node["attrs"]
                .as_object()
                .map(|attrs| attrs.keys().cloned().collect())
                .unwrap_or_default();
            if node["blockId"].as_bool() == Some(true) {
                let _new = attrs.insert("blockId".to_owned());
            }
            if node["inline"].as_bool() == Some(true) {
                let _new = attrs.insert("marks".to_owned());
            }
            (node["name"].as_str().unwrap().to_owned(), attrs)
        })
        .collect()
}

/// Every node, attribute and mark the codec writes is in the editor's
/// schema.
fn check_schema(
    nodes: &[Node],
    schema: &HashMap<String, HashSet<String>>,
    out: &mut Vec<String>,
) {
    for node in nodes {
        match node {
            Node::Element(element) => {
                if NodeType::from_name(&element.name).is_none() {
                    out.push(format!("node {}", element.name));
                }
                let allowed = schema.get(&element.name);
                out.extend(
                    element
                        .attrs
                        .keys()
                        .filter(|key| {
                            !allowed
                                .is_some_and(|allowed| allowed.contains(*key))
                        })
                        .map(|key| format!("{}.{key}", element.name)),
                );
                check_schema(&element.children, schema, out);
            }
            Node::Text(runs) => out.extend(
                runs.iter()
                    .flat_map(|run| run.marks.keys())
                    .filter(|mark| MarkType::from_name(mark).is_none())
                    .map(|mark| format!("mark {mark}")),
            ),
        }
    }
}

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

fn inputs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> =
        entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        if path.is_dir() {
            inputs(&path, out);
        } else if path.extension().is_some_and(|e| e == "md")
            && name != "README.md"
        {
            out.push(path);
        }
    }
}

/// Markdown → tree → Yjs → tree → Markdown.
fn through_yjs(markdown: &str) -> String {
    let doc = tree::new_doc();
    tree::write(&doc, &FormatCodec.parse_markdown(markdown).unwrap());
    FormatCodec.to_markdown(&tree::read(&doc)).unwrap()
}

#[test]
fn corpus_survives_the_editor_tree() {
    let mut files = Vec::new();
    for root in ["format", "guide"] {
        inputs(&corpus_root().join(root), &mut files);
    }
    inputs(&corpus_root().join("../docs"), &mut files);
    assert!(files.len() > 50, "corpus looks empty: {}", files.len());

    let schema = schema_attrs();
    let mut failures = Vec::new();
    for input in &files {
        let name = input
            .strip_prefix(corpus_root())
            .unwrap_or(input)
            .display()
            .to_string();
        let source = fs::read_to_string(input).unwrap();
        let parsed = vellum_format::parse(&source).document;
        let canonical = vellum_format::serialize(&parsed);

        // The tree conversion alone is lossless on the AST.
        let tree = from_document(&parsed);
        let mut unknown = Vec::new();
        check_schema(&tree.content, &schema, &mut unknown);
        if !unknown.is_empty() {
            failures
                .push(format!("{name}: names outside the schema: {unknown:?}"));
        }
        let back = to_document(&tree);
        if back != parsed {
            failures.push(format!(
                "{name}: AST changed\n  before: {parsed:?}\n  after:  {back:?}"
            ));
            continue;
        }
        let markdown = through_yjs(&source);
        if markdown != canonical {
            failures.push(format!(
                "{name}: Markdown differs:\n{markdown}\n--- expected:\n\
                    {canonical}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Markdown made of the constructs the editor produces most.
fn block() -> impl Strategy<Value = String> {
    let word = "[a-z]{1,6}";
    let inline = prop_oneof![
        word.prop_map(|w| w),
        word.prop_map(|w| format!("**{w}**")),
        word.prop_map(|w| format!("*{w}*")),
        word.prop_map(|w| format!("`{w}`")),
        word.prop_map(|w| format!("~~{w}~~")),
        word.prop_map(|w| format!("[{w}](https://northstar.dev/{w})")),
        word.prop_map(|w| format!("**a *{w}* b**")),
        word.prop_map(|w| format!("@{w}")),
        word.prop_map(|w| format!("[[{w}]]")),
    ];
    let line =
        prop::collection::vec(inline, 1..5).prop_map(|parts| parts.join(" "));
    prop_oneof![
        line.clone(),
        (1..=6_usize, line.clone())
            .prop_map(|(level, text)| format!("{} {text}", "#".repeat(level))),
        prop::collection::vec(line.clone(), 1..4).prop_map(|items| items
            .iter()
            .map(|i| format!("- {i}"))
            .collect::<Vec<_>>()
            .join("\n")),
        prop::collection::vec((any::<bool>(), line.clone()), 1..4).prop_map(
            |items| items
                .iter()
                .map(|(done, i)| format!(
                    "- [{}] {i}",
                    if *done { "x" } else { " " }
                ))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        line.clone().prop_map(|text| format!("> {text}")),
        "[a-z ]{0,20}".prop_map(|code| format!("```rust\n{code}\n```")),
        line.clone()
            .prop_map(|text| format!("::: info\n{text}\n:::")),
        (line.clone(), line).prop_map(|(a, b)| format!(
            "| {a} | {b} |\n| --- | :-: |\n| {b} | {a} |"
        )),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Any document comes back from the editor tree as its canonical form,
    /// and that form is a fixed point.
    #[test]
    fn documents_round_trip(
        blocks in prop::collection::vec(block(), 0..6),
        title in prop::option::of("[A-Za-z ]{1,12}"),
    ) {
        let mut source = title.map_or_else(String::new, |title| {
            format!("---\ntitle: \"{title}\"\ntags: [a, b]\n---\n\n")
        });
        source.push_str(&blocks.join("\n\n"));
        source.push('\n');
        let canonical = vellum_format::serialize(
            &vellum_format::parse(&source).document
        );
        let once = through_yjs(&source);
        prop_assert_eq!(&once, &canonical);
        prop_assert_eq!(through_yjs(&once), once);
    }
}
