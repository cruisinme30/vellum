// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Export goldens: every page of the format and guide corpus is exported
//! to PDF and Word, and the PDF's text (page by page) and the Word file's
//! structure (paragraph styles, list levels, tables) are compared with
//! `corpus/export/<same path>.pdf.txt` and `.docx.txt`.
//!
//! Run with `VELLUM_BLESS=1` to rewrite the goldens after an intended
//! change, then review the diff.

#![allow(clippy::unwrap_used, reason = "test helpers fail loudly")]

use std::fmt::Write as _;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use quick_xml::Reader;
use quick_xml::events::Event;
use vellum_export::docx::{DocxOptions, write};
use vellum_export::pdf::{PdfEngine, PrintTheme};
use vellum_export::{LowerOptions, Ports, Target, lower};
use vellum_jobs::export::CollectionsEngine;

const ROOTS: [&str; 2] = ["format", "guide"];

fn corpus() -> PathBuf {
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
            && !name.ends_with(".canonical.md")
            && name != "README.md"
        {
            out.push(path);
        }
    }
}

/// The Word file's structure: one line per paragraph with its style and
/// list level, and tables as rows of cells.
fn docx_structure(bytes: &[u8]) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    let _ = zip
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    let mut reader = Reader::from_str(&xml);
    let mut out = String::new();
    let (mut style, mut level, mut text) =
        (String::new(), None::<String>, String::new());
    let (mut tables, mut row, mut cell): (usize, Vec<String>, String) =
        (0, Vec::new(), String::new());
    let mut in_instr = false;
    loop {
        match reader.read_event().unwrap() {
            Event::Start(e) | Event::Empty(e) => {
                let val = || {
                    e.attributes()
                        .flatten()
                        .find(|a| a.key.local_name().as_ref() == "val")
                        .map(|a| a.value.to_string())
                };
                match e.local_name().as_ref() {
                    "p" => {
                        style.clear();
                        level = None;
                        text.clear();
                    }
                    "pStyle" => style = val().unwrap_or_default(),
                    "ilvl" => level = val(),
                    "tbl" => tables += 1,
                    "tr" if tables == 1 => row.clear(),
                    "tc" if tables == 1 => cell.clear(),
                    "drawing" => text.push_str("[picture]"),
                    "oMath" => text.push_str("[equation]"),
                    "instrText" => in_instr = true,
                    "tab" => text.push(' '),
                    _ => {}
                }
            }
            Event::Text(t) if !in_instr => text.push_str(&t.xml10_content()),
            Event::GeneralRef(g) => {
                let name = g.xml10_content();
                text.push_str(match name.as_ref() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    _ => "'",
                });
            }
            Event::End(e) => match e.local_name().as_ref() {
                "instrText" => in_instr = false,
                "p" if tables > 0 => {
                    if !text.trim().is_empty() {
                        if !cell.is_empty() {
                            cell.push(' ');
                        }
                        cell.push_str(text.trim());
                    }
                    text.clear();
                }
                "p" => {
                    if !text.trim().is_empty() || !style.is_empty() {
                        let style =
                            if style.is_empty() { "Normal" } else { &style };
                        let level = level
                            .as_ref()
                            .map(|l| format!("/{l}"))
                            .unwrap_or_default();
                        let _ =
                            writeln!(out, "[{style}{level}] {}", text.trim());
                    }
                }
                "tc" if tables == 1 => row.push(std::mem::take(&mut cell)),
                "tr" if tables == 1 => {
                    let _ = writeln!(out, "| {} |", row.join(" | "));
                }
                "tbl" => {
                    tables = tables.saturating_sub(1);
                    if tables == 0 {
                        out.push('\n');
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// Ports with the collections engine, so calculators show their values.
fn ports() -> Ports {
    Ports {
        collections: std::sync::Arc::new(CollectionsEngine),
        ..Ports::default()
    }
}

#[tokio::test]
async fn corpus_exports_match_their_goldens() {
    let bless = std::env::var_os("VELLUM_BLESS").is_some();
    let mut files = Vec::new();
    for root in ROOTS {
        inputs(&corpus().join(root), &mut files);
    }
    assert!(files.len() >= 20, "the corpus looks empty: {}", files.len());
    let engine = PdfEngine::new();
    let theme = PrintTheme::default();
    let ports = ports();
    let mut failures = Vec::new();
    for input in &files {
        let source = fs::read_to_string(input).unwrap();
        let doc = vellum_format::parse(&source).document;
        let rel = input.strip_prefix(corpus()).unwrap().with_extension("");
        let golden = corpus().join("export").join(&rel);

        let print = lower(
            &doc,
            &ports,
            &LowerOptions::new(Target::Print, "2026-10-29"),
        )
        .await;
        let pdf = match engine.render(&print.flow, &theme) {
            Ok(pdf) => pdf.pages.join("\n\u{c}\n") + "\n",
            Err(error) => format!("ERROR {error}\n"),
        };
        let office = lower(
            &doc,
            &ports,
            &LowerOptions::new(Target::Office, "2026-10-29"),
        )
        .await;
        let docx = docx_structure(
            &write(&office.flow, &DocxOptions::default()).unwrap(),
        );

        for (suffix, actual) in [(".pdf.txt", pdf), (".docx.txt", docx)] {
            let path = PathBuf::from(format!("{}{suffix}", golden.display()));
            if bless {
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(&path, &actual).unwrap();
                continue;
            }
            let expected = fs::read_to_string(&path).unwrap_or_default();
            if expected != actual {
                failures.push(format!("{}", path.display()));
            }
        }
        assert!(
            !format!("{:?}", print.flow).is_empty(),
            "{}",
            input.display()
        );
    }
    assert!(
        failures.is_empty(),
        "{} export goldens differ (run with VELLUM_BLESS=1 and review):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
