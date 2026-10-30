// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Book export: scopes, heading levels, the regenerated contents and
//! cross-references that print with page numbers.

use std::io::Read as _;

use vellum_export::book::{BookExport, BookItem, BookScope, BookSection, lower_book};
use vellum_export::docx::{DocxOptions, write};
use vellum_export::flow::{Node, Span};
use vellum_export::pdf::{PdfEngine, PrintTheme};
use vellum_export::{LowerOptions, Ports, Target};

fn section(
    id: &str,
    number: &str,
    title: &str,
    depth: usize,
    body: &str,
    children: Vec<BookSection>,
) -> BookSection {
    BookSection {
        id: Some(id.to_owned()),
        title: title.to_owned(),
        number_label: Some(number.to_owned()),
        label: format!("§{number} {title}"),
        depth,
        draft: false,
        document: Some(vellum_format::parse(body).document),
        children,
    }
}

fn handbook(scope: BookScope) -> BookExport {
    let services = section(
        "doc-services",
        "2.1",
        "Services",
        2,
        "# Services\n\n## Gateway\n\nThe gateway terminates TLS.\n",
        Vec::new(),
    );
    let architecture = section(
        "doc-arch",
        "2",
        "Architecture",
        1,
        "# Architecture\n\nSee [](vellum://doc/doc-intro) first.\n",
        vec![services],
    );
    let intro = section(
        "doc-intro",
        "1",
        "Introduction",
        1,
        "# Introduction\n\nThe services are in [§](vellum://doc/doc-services). \
         Pages outside the book: [Runbook](vellum://doc/doc-runbook).\n",
        Vec::new(),
    );
    let draft = BookSection {
        id: None,
        title: "Roadmap".into(),
        number_label: None,
        label: "Roadmap".into(),
        depth: 1,
        draft: true,
        document: None,
        children: Vec::new(),
    };
    BookExport {
        title: "Northstar handbook".into(),
        items: vec![
            BookItem::Section(Box::new(intro)),
            BookItem::Part("Design".into()),
            BookItem::Section(Box::new(architecture)),
            BookItem::Separator,
            BookItem::Section(Box::new(draft)),
        ],
        scope,
        toc_depth: 3,
    }
}

fn headings(blocks: &[Node]) -> Vec<(u8, Option<String>, String)> {
    blocks
        .iter()
        .filter_map(|node| match node {
            Node::Heading {
                level,
                number,
                spans,
                ..
            } => Some((
                *level,
                number.clone(),
                vellum_export::flow::plain_text(spans),
            )),
            _ => None,
        })
        .collect()
}

fn xrefs(blocks: &[Node]) -> Vec<String> {
    let mut out = Vec::new();
    for node in blocks {
        let mut node = node.clone();
        node.spans_mut(&mut |spans| {
            for span in spans.iter() {
                if let Span::Xref { text, .. } = span {
                    out.push(text.clone());
                }
            }
        });
    }
    out
}

fn options(target: Target) -> LowerOptions {
    LowerOptions::new(target, "2026-10-29")
}

#[tokio::test]
async fn whole_book_maps_depth_to_heading_level() {
    let lowered = lower_book(
        &handbook(BookScope::Whole),
        &Ports::default(),
        &options(Target::Print),
    )
    .await;
    let flow = &lowered.flow;
    assert_eq!(flow.title, "Northstar handbook");
    assert!(matches!(flow.blocks.first(), Some(Node::Toc { depth: 3 })));
    assert_eq!(
        headings(&flow.blocks),
        [
            (1, Some("1".into()), "Introduction".into()),
            (1, None, "Design".into()),
            (1, Some("2".into()), "Architecture".into()),
            (2, Some("2.1".into()), "Services".into()),
            (3, None, "Gateway".into()),
        ]
    );
    // `§` takes the label; other text is kept; links outside the book
    // are plain text without a base URL.
    assert_eq!(xrefs(&flow.blocks), ["§2.1 Services", "§1 Introduction"]);
    let text = format!("{:?}", flow.blocks);
    assert!(text.contains("Runbook") && !text.contains("doc-runbook"));
}

#[tokio::test]
async fn subtree_and_section_are_rebased() {
    let subtree = lower_book(
        &handbook(BookScope::Subtree("doc-arch".into())),
        &Ports::default(),
        &options(Target::Print),
    )
    .await;
    assert_eq!(subtree.flow.title, "Northstar handbook — Architecture");
    assert_eq!(
        headings(&subtree.flow.blocks),
        [
            (1, Some("2".into()), "Architecture".into()),
            (2, Some("2.1".into()), "Services".into()),
            (3, None, "Gateway".into()),
        ]
    );
    // The introduction is not in this export, so its reference is text.
    assert!(xrefs(&subtree.flow.blocks).is_empty());

    let one = lower_book(
        &handbook(BookScope::Section("doc-services".into())),
        &Ports::default(),
        &options(Target::Print),
    )
    .await;
    assert_eq!(
        headings(&one.flow.blocks),
        [
            (1, Some("2.1".into()), "Services".into()),
            (2, None, "Gateway".into()),
        ]
    );

    let missing = lower_book(
        &handbook(BookScope::Section("nope".into())),
        &Ports::default(),
        &options(Target::Print),
    )
    .await;
    assert_eq!(missing.warnings.len(), 1);
}

#[tokio::test]
async fn pdf_cross_references_print_page_numbers() {
    let lowered = lower_book(
        &handbook(BookScope::Whole),
        &Ports::default(),
        &options(Target::Print),
    )
    .await;
    let pdf = PdfEngine::new()
        .render(&lowered.flow, &PrintTheme::default())
        .unwrap();
    let all = pdf.pages.join("\n");
    assert!(all.contains("Contents"), "{all}");
    let services_page = pdf
        .pages
        .iter()
        .position(|p| p.contains("The gateway terminates TLS"))
        .unwrap()
        + 1;
    assert!(
        all.contains(&format!("§2.1 Services, p.\u{a0}{services_page}")),
        "{all}"
    );
}

#[tokio::test]
async fn word_cross_references_use_page_fields() {
    let lowered = lower_book(
        &handbook(BookScope::Whole),
        &Ports::default(),
        &options(Target::Office),
    )
    .await;
    let bytes = write(&lowered.flow, &DocxOptions::default()).unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut xml = String::new();
    let _ = zip
        .by_name("word/document.xml")
        .unwrap()
        .read_to_string(&mut xml)
        .unwrap();
    assert!(xml.contains("PAGEREF"), "{xml}");
    assert!(xml.contains(r#"TOC \o "1-3""#));
    assert!(xml.contains("§2.1 Services"));
}
