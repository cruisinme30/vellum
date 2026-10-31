// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The golden corpus: every `.md` under `corpus/format/` and
//! `corpus/guide/` is parsed and compared with its `.ast.json`; serializing
//! must give the input back (or its `.canonical.md`), and parsing that must
//! give the same tree.
//!
//! Run with `VELLUM_BLESS=1` to write the expected files after a reviewed
//! change.

use std::fs;
use std::path::{Path, PathBuf};

use vellum_format::{parse, serialize};

const ROOTS: [&str; 2] = ["format", "guide"];

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
            .unwrap_or_default();
        if path.is_dir() {
            inputs(&path, out);
        } else if path.extension().is_some_and(|e| e == "md")
            && !name.ends_with(".canonical.md")
            && name != "README.md"
        {
            out.push(path);
        }
    }
}

fn sibling(input: &Path, suffix: &str) -> PathBuf {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    input.with_file_name(format!("{stem}{suffix}"))
}

#[test]
fn golden_corpus_round_trips() {
    let bless = std::env::var_os("VELLUM_BLESS").is_some();
    let mut files = Vec::new();
    for root in ROOTS {
        inputs(&corpus_root().join(root), &mut files);
    }
    assert!(
        files.len() >= 20,
        "corpus/format looks empty: {} files",
        files.len()
    );

    let mut failures = Vec::new();
    for input in &files {
        let source = fs::read_to_string(input).unwrap();
        let parsed = parse(&source);
        let ast = serde_json::to_string_pretty(&parsed).unwrap() + "\n";
        let canonical = serialize(&parsed.document);
        let ast_path = sibling(input, ".ast.json");
        let canonical_path = sibling(input, ".canonical.md");

        if bless {
            fs::write(&ast_path, &ast).unwrap();
            if canonical == source {
                let _ = fs::remove_file(&canonical_path);
            } else {
                fs::write(&canonical_path, &canonical).unwrap();
            }
        }

        let name = input
            .strip_prefix(corpus_root())
            .unwrap_or(input)
            .display()
            .to_string();
        let expected_ast = fs::read_to_string(&ast_path).unwrap_or_default();
        if ast != expected_ast {
            failures.push(format!(
                "{name}: AST differs from {}",
                ast_path.display()
            ));
        }
        let expected = fs::read_to_string(&canonical_path)
            .unwrap_or_else(|_| source.clone());
        if canonical != expected {
            failures
                .push(format!("{name}: canonical form differs:\n{canonical}"));
        }
        if parse(&canonical).document != parsed.document {
            failures.push(format!(
                "{name}: canonical form parses to a different tree"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} corpus failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Every Vellum Markdown document in the repository round-trips too.
#[test]
fn repository_documents_round_trip() {
    let mut files = Vec::new();
    inputs(&corpus_root().join("../docs"), &mut files);
    let mut failures = Vec::new();
    for input in &files {
        let source = fs::read_to_string(input).unwrap();
        let document = parse(&source).document;
        let canonical = serialize(&document);
        if parse(&canonical).document != document
            || serialize(&parse(&canonical).document) != canonical
        {
            failures.push(input.display().to_string());
        }
    }
    assert!(
        failures.is_empty(),
        "documents that do not round-trip: {failures:?}"
    );
}
