// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

use std::io::{Cursor, Write as _};

use zip::write::SimpleFileOptions;

use super::*;
use crate::flow::{
    Asset, Flow, Item, Layout, Media, Node, Span, Table as FlowTable,
    TableStyle,
};

/// Imports without a collections engine; `tests/it/import.rs` covers the
/// collections it builds.
fn import(
    source: Source,
    name: &str,
    bytes: &[u8],
) -> Result<ImportSet, ImportError> {
    super::import(source, name, bytes, &NoCollections)
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut out);
    for (name, bytes) in files {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
    }
    let _ = zip.finish().unwrap();
    out.into_inner()
}

fn page<'a>(set: &'a ImportSet, path: &str) -> (&'a str, String) {
    let file = set
        .file(path)
        .unwrap_or_else(|| panic!("no {path} in {:?}", paths(set)));
    let FileKind::Page { id, .. } = &file.kind else {
        panic!("{path} is not a page");
    };
    (id.as_str(), file.text())
}

fn paths(set: &ImportSet) -> Vec<&str> {
    set.files.iter().map(|f| f.path.as_str()).collect()
}

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\
        \0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89";

#[test]
fn markdown_folders_get_ids_and_id_links() {
    let set = import(
        Source::Markdown,
        "wiki.zip",
        &zip_of(&[
            (
                "Wiki/Home.md",
                b"# Home\n\n\
                  See [restart](Runbooks/Restart%20gateway.md#steps), \
                  ![diagram](assets/diagram.png) and [missing](Nope.md).\n",
            ),
            (
                "Wiki/Runbooks/Restart gateway.md",
                b"---\ntitle: Restart the gateway\ntags: [ops]\n---\n\nBody.\n",
            ),
            ("Wiki/assets/diagram.png", PNG),
        ]),
    )
    .unwrap();
    let (restart_id, restart) = page(&set, "Runbooks/Restart gateway.md");
    let (home_id, home) = page(&set, "Home.md");
    assert_ne!(home_id, restart_id);
    assert!(restart_id.parse::<DocId>().is_ok(), "{restart_id}");
    assert!(
        restart.starts_with(&format!(
            "---\nid: {restart_id}\ntitle: Restart the gateway\ntags:"
        )),
        "{restart}"
    );
    assert!(restart.contains("vellum: 1"), "{restart}");
    assert!(
        home.contains(&format!("[restart](vellum://doc/{restart_id}#steps)")),
        "{home}"
    );
    assert!(home.contains("![diagram](assets/diagram.png)"), "{home}");
    assert!(set.file("assets/diagram.png").is_some());
    assert_eq!(set.warnings.len(), 1, "{:?}", set.warnings);
    assert!(set.warnings[0].message.contains("Nope.md"));
}

#[test]
fn obsidian_wiki_links_and_embeds_resolve() {
    let set = import(
        Source::Obsidian,
        "vault.zip",
        &zip_of(&[
            (
                "vault/Daily.md",
                b"Read [[Gateway notes|the notes]]. ![[flow.png]]\n",
            ),
            ("vault/Topics/Gateway notes.md", b"# Gateway notes\n"),
            ("vault/attachments/flow.png", PNG),
            ("vault/.obsidian/app.json", b"{}"),
        ]),
    )
    .unwrap();
    let (gateway_id, _) = page(&set, "Topics/Gateway notes.md");
    let (_, daily) = page(&set, "Daily.md");
    assert!(
        daily.contains(&format!("[the notes](vellum://doc/{gateway_id})")),
        "{daily}"
    );
    assert!(daily.contains("![](attachments/flow.png)"), "{daily}");
    assert!(!paths(&set).iter().any(|p| p.contains("obsidian")));
}

#[test]
fn outline_and_docmost_notices_become_callouts() {
    for source in [Source::Outline, Source::Docmost] {
        let set = import(
            source,
            "export.zip",
            &zip_of(&[(
                "Engineering/Deploys.md",
                b"# Deploys\n\n:::warning\nFreeze on Fridays.\n:::\n",
            )]),
        )
        .unwrap();
        let (_, text) = page(&set, "Deploys.md");
        assert!(
            text.contains("::: warn\n\nFreeze on Fridays.\n\n:::"),
            "{text}"
        );
    }
}

#[test]
fn notion_databases_without_an_engine_stay_csv_and_pages() {
    let home = "Home 0123456789abcdef0123456789abcdef";
    let tasks = "Tasks 11111111111111111111111111111111";
    let row = "Rotate keys 22222222222222222222222222222222";
    let home_md = format!(
        "# Home\n\nTrack work in [Rotate keys]({}/{}/{}.md).\n",
        home.replace(' ', "%20"),
        tasks.replace(' ', "%20"),
        row.replace(' ', "%20")
    );
    let csv = "Name,Status,Estimate\nRotate keys,Done,3\nAudit logs,Todo,5\n";
    let row_text = "# Rotate keys\n\nStatus: Done\nEstimate: 3\n\n\
        Rotate the signing keys.\n";
    let set = import(
        Source::Notion,
        "notion.zip",
        &zip_of(&[
            (&format!("{home}.md"), home_md.as_bytes()),
            (&format!("{home}/{tasks}_all.csv"), csv.as_bytes()),
            (&format!("{home}/{tasks}/{row}.md"), row_text.as_bytes()),
        ]),
    )
    .unwrap();
    let all = paths(&set);
    assert!(all.contains(&"Home/Tasks.csv"), "{all:?}");
    assert!(
        set.file("Home/Tasks.csv")
            .unwrap()
            .text()
            .contains("Audit logs")
    );
    let rotate = set
        .pages()
        .find(|f| f.text().contains("Rotate the signing keys."))
        .unwrap()
        .text();
    let row_id = rotate
        .lines()
        .find_map(|l| l.strip_prefix("id: "))
        .unwrap()
        .to_owned();
    let (_, home) = page(&set, "Home.md");
    assert!(home.contains(&format!("vellum://doc/{row_id}")), "{home}");
    assert!(
        set.warnings
            .iter()
            .any(|w| w.message.contains("not a collection")),
        "{:?}",
        set.warnings
    );
}

#[test]
fn confluence_spaces_keep_their_page_tree() {
    let index = br#"<html><body><div class="pageSection"><ul>
        <li><a href="Home_1.html">Home</a><ul><li><a href="Restart_2.html">Restart</a></li></ul></li>
        </ul></div></body></html>"#;
    let home = br#"<html><head><title>Ops : Home</title></head><body>
        <div id="main-header"><div id="breadcrumb-section">Ops</div><h1 id="title-heading"><span id="title-text">Ops : Home</span></h1></div>
        <div id="main-content"><p>Start with <a href="Restart_2.html">the restart page</a>.</p></div>
        <div id="footer">Generated</div></body></html>"#;
    let restart = br#"<html><body><span id="title-text">Ops : Restart the gateway</span>
        <div id="main-content"><p><img src="attachments/2/diagram.png" alt="Request path"></p>
        <div class="confluence-information-macro confluence-information-macro-note"><div class="confluence-information-macro-body"><p>Drain first.</p></div></div>
        </div></body></html>"#;
    let set = import(
        Source::Confluence,
        "ops.zip",
        &zip_of(&[
            ("OPS/index.html", index),
            ("OPS/Home_1.html", home),
            ("OPS/Restart_2.html", restart),
            ("OPS/attachments/2/diagram.png", PNG),
            ("OPS/styles/site.css", b"body{}"),
            ("OPS/images/icons/bullet.png", PNG),
        ]),
    )
    .unwrap();
    let (restart_id, restart) = page(&set, "Home/Restart the gateway.md");
    let (_, home) = page(&set, "Home.md");
    assert!(home.contains("# Home"), "{home}");
    assert!(
        !home.contains("Generated") && !home.contains("breadcrumb"),
        "{home}"
    );
    assert!(
        home.contains(&format!(
            "[the restart page](vellum://doc/{restart_id})"
        )),
        "{home}"
    );
    assert!(
        restart.contains("![Request path](../attachments/2/diagram.png)"),
        "{restart}"
    );
    assert!(
        restart.contains("::: warn\n\nDrain first.\n\n:::"),
        "{restart}"
    );
    let all = paths(&set);
    assert!(
        !all.iter()
            .any(|p| p.contains("styles") || p.contains("icons")),
        "{all:?}"
    );
}

/// A Word file written by the export writer reads back as the same
/// structure.
#[test]
#[allow(clippy::too_many_lines)] // the document is spelled out
fn word_round_trips_through_the_writer() {
    let flow = Flow {
        title: "Gateway".into(),
        as_of: "2026-10-29".into(),
        layout: Layout::Document,
        show_title: false,
        blocks: vec![
            Node::Heading {
                level: 1,
                number: None,
                spans: vec![Span::plain("Gateway")],
                anchor: None,
                outlined: true,
            },
            Node::Paragraph {
                spans: vec![
                    Span::plain("Read the "),
                    Span::bold("runbook"),
                    Span::plain(" at "),
                    Span::Link {
                        url: "https://northstar.dev/runbook".into(),
                        spans: vec![Span::plain("northstar.dev")],
                    },
                    Span::plain("."),
                ],
                anchor: None,
            },
            Node::List {
                start: Some(1),
                items: vec![
                    Item {
                        check: None,
                        blocks: vec![Node::Paragraph {
                            spans: vec![Span::plain("Drain")],
                            anchor: None,
                        }],
                    },
                    Item {
                        check: None,
                        blocks: vec![
                            Node::Paragraph {
                                spans: vec![Span::plain("Restart")],
                                anchor: None,
                            },
                            Node::List {
                                start: None,
                                items: vec![Item {
                                    check: None,
                                    blocks: vec![Node::Paragraph {
                                        spans: vec![Span::plain(
                                            "Watch the error rate",
                                        )],
                                        anchor: None,
                                    }],
                                }],
                            },
                        ],
                    },
                ],
            },
            Node::Table(FlowTable {
                style: TableStyle::Data,
                align: vec![crate::flow::Align::Left, crate::flow::Align::Left],
                header: vec![
                    crate::flow::Cell::text("Step"),
                    crate::flow::Cell::text("Owner"),
                ],
                rows: vec![vec![
                    crate::flow::Cell::text("Drain"),
                    crate::flow::Cell::text("Priya"),
                ]],
            }),
            Node::Code {
                lang: "bash".into(),
                text: "vellum restart --drain".into(),
            },
            Node::Image {
                asset: "a0.png".into(),
                alt: "Request path".into(),
                caption: String::new(),
            },
        ],
        assets: vec![Asset {
            name: "a0.png".into(),
            media: Media::Png,
            bytes: PNG.to_vec(),
            width: Some(1),
            height: Some(1),
        }],
    };
    let bytes = crate::docx::write(&flow, &crate::docx::DocxOptions::default())
        .unwrap();
    let set = import(Source::Word, "Gateway runbook.docx", &bytes).unwrap();
    let (_, text) = page(&set, "Gateway runbook.md");
    assert!(text.contains("title: Gateway\n"), "{text}");
    assert!(text.contains("# Gateway\n"), "{text}");
    assert!(
        text.contains(
            "Read the **runbook** at \
            [northstar.dev](https://northstar.dev/runbook)."
        ),
        "{text}"
    );
    assert!(
        text.contains("1. Drain\n2. Restart\n   - Watch the error rate"),
        "{text}"
    );
    assert!(text.contains("| Step | Owner |"), "{text}");
    assert!(text.contains("vellum restart --drain"), "{text}");
    assert!(
        text.contains("![Request path](Gateway%20runbook%20assets/"),
        "{text}"
    );
    assert!(
        set.files.iter().any(|f| f.kind == FileKind::Asset
            && f.path.starts_with("Gateway runbook assets/")),
        "{:?}",
        paths(&set)
    );
}

#[test]
fn bad_sources_are_format_errors() {
    assert!(matches!(
        import(Source::Word, "x.docx", b"not a zip"),
        Err(ImportError::Format { .. })
    ));
    assert!(matches!(
        import(Source::Markdown, "x.zip", &zip_of(&[("a.png", PNG)])),
        Err(ImportError::Format { .. })
    ));
    assert!(matches!(
        import(
            Source::Word,
            "x.docx",
            &zip_of(&[("word/other.xml", b"<x/>")])
        ),
        Err(ImportError::Format { .. })
    ));
}

#[test]
fn output_paths_are_safe_and_unique() {
    assert_eq!(clean_path("a:b/c?d.md"), "a-b/c-d.md");
    assert_eq!(clean_path(".hidden/x"), "untitled.hidden/x");
    let long = "x".repeat(300);
    assert!(clean_path(&format!("{long}.md")).len() <= MAX_NAME_BYTES);
    let set = import(
        Source::Markdown,
        "x.zip",
        &zip_of(&[("Page.md", b"# A\n"), ("page.MD", b"# B\n")]),
    )
    .unwrap();
    let mut all = paths(&set);
    all.sort_unstable();
    assert_eq!(all, ["Page.md", "page 2.md"]);
}
