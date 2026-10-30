// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Markdown and zip export.
//!
//! A Markdown export is the page's own Vellum Markdown with live blocks
//! frozen: a `collection` embed becomes a table of the rows it showed,
//! titled with the date, and a chart drawn from a collection
//! (`source=`) gets its current values written in. Everything else is
//! already plain text. Several pages, or a page with its attachments,
//! go out as a zip; entries are written in the order given with fixed
//! timestamps, so the same export gives the same bytes.

use std::io::{Cursor, Write as _};
use std::time::Instant;

use vellum_format::blocks::Chart;
use vellum_format::{Alignment, Block, Document, FenceKind, Inline, Table};
use zip::CompressionMethod;
use zip::write::{SimpleFileOptions, ZipWriter};

use crate::error::{ExportError, ExportWarning};
use crate::metrics;
use crate::ports::Ports;

/// Most entries in one zip export.
pub const MAX_ENTRIES: usize = 50_000;

/// A Markdown export: the text and what could not be frozen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownExport {
    /// Canonical Vellum Markdown.
    pub text: String,
    /// Live blocks left as written because their data was unavailable.
    pub warnings: Vec<ExportWarning>,
}

/// Writes `doc` as Markdown with live blocks frozen at `as_of`.
#[tracing::instrument(name = "export.markdown", skip_all)]
pub async fn markdown(
    doc: &Document,
    ports: &Ports,
    as_of: &str,
) -> MarkdownExport {
    let started = Instant::now();
    let mut doc = doc.clone();
    let mut warnings = Vec::new();
    freeze(&mut doc.blocks, ports, as_of, &mut warnings).await;
    let text = vellum_format::serialize(&doc);
    metrics::export_done("markdown", started, Ok(text.len()));
    MarkdownExport { text, warnings }
}

async fn freeze(
    blocks: &mut [Block],
    ports: &Ports,
    as_of: &str,
    warnings: &mut Vec<ExportWarning>,
) {
    // Iterative over nesting so the future stays small and non-recursive.
    let mut stack: Vec<&mut [Block]> = vec![blocks];
    while let Some(blocks) = stack.pop() {
        for block in blocks.iter_mut() {
            match block {
                Block::Fence(fence)
                    if fence.kind() == FenceKind::Collection =>
                {
                    match ports.live.collection(fence).await {
                        Ok(snapshot) => {
                            *block = snapshot_block(&snapshot, as_of)
                        }
                        Err(error) => warnings.push(ExportWarning::new(
                            "collection",
                            format!("{}; kept as an embed", error.0),
                        )),
                    }
                }
                Block::Fence(fence)
                    if fence.kind() == FenceKind::Chart
                        && Chart::parse(fence).data.source.is_some() =>
                {
                    match ports.live.chart(fence).await {
                        Ok(resolved) => *fence = resolved,
                        Err(error) => warnings.push(ExportWarning::new(
                            "chart",
                            format!("{}; kept with its source", error.0),
                        )),
                    }
                }
                Block::BlockQuote { blocks, .. } => stack.push(blocks),
                Block::List(list) => {
                    for item in &mut list.items {
                        stack.push(&mut item.blocks);
                    }
                }
                Block::Container(container) => {
                    for section in &mut container.sections {
                        stack.push(section);
                    }
                }
                _ => {}
            }
        }
    }
}

/// A frozen collection: a titled table, or a note when it was empty.
fn snapshot_block(snapshot: &crate::ports::Snapshot, as_of: &str) -> Block {
    let title = if snapshot.title.is_empty() {
        format!("As of {as_of}")
    } else {
        format!("{} (as of {as_of})", snapshot.title)
    };
    let text = |s: &String| vec![Inline::Text { text: s.clone() }];
    let caption = Block::Paragraph {
        content: vec![Inline::Emphasis {
            content: vec![Inline::Text { text: title }],
        }],
        id: None,
    };
    if snapshot.columns.is_empty() {
        return caption;
    }
    let table = Block::Table(Table {
        id: None,
        alignments: vec![Alignment::None; snapshot.columns.len()],
        header: snapshot.columns.iter().map(text).collect(),
        rows: snapshot
            .rows
            .iter()
            .map(|row| {
                let mut cells: Vec<Vec<Inline>> =
                    row.iter().map(text).collect();
                cells.resize(snapshot.columns.len(), Vec::new());
                cells
            })
            .collect(),
    });
    Block::Container(vellum_format::Container {
        id: None,
        name: "note".into(),
        arg: String::new(),
        attrs: vellum_format::Attributes::default(),
        sections: vec![vec![caption, table]],
    })
}

/// Packs `files` (`path`, bytes) into a zip. Text formats are deflated and
/// already-compressed media stored.
///
/// # Errors
///
/// [`ExportError::Invalid`] for a path that is empty, absolute or climbs
/// with `..`, or a repeated path; [`ExportError::TooLarge`] past
/// [`MAX_ENTRIES`].
#[tracing::instrument(
    name = "export.zip",
    skip_all,
    fields(files = files.len())
)]
pub fn zip(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, ExportError> {
    let started = Instant::now();
    let result = zip_inner(files);
    metrics::export_done("zip", started, result.as_ref().map(Vec::len));
    result
}

fn zip_inner(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, ExportError> {
    if files.len() > MAX_ENTRIES {
        return Err(ExportError::TooLarge(format!(
            "more than {MAX_ENTRIES} files"
        )));
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(&mut out);
    for (path, bytes) in files {
        let bad = path.is_empty()
            || path.starts_with('/')
            || path.contains('\\')
            || path
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..");
        if bad || !seen.insert(path.as_str()) {
            return Err(ExportError::Invalid(format!(
                "bad or repeated zip path `{path}`"
            )));
        }
        let lower = path.to_ascii_lowercase();
        let stored = [
            ".png", ".jpg", ".jpeg", ".gif", ".webp", ".zip", ".pdf", ".docx",
        ]
        .iter()
        .any(|ext| lower.ends_with(ext));
        let method = if stored {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        zip.start_file(
            path.as_str(),
            SimpleFileOptions::default().compression_method(method),
        )
        .map_err(|e| ExportError::Write(e.to_string()))?;
        zip.write_all(bytes)
            .map_err(|e| ExportError::Write(e.to_string()))?;
    }
    let _ = zip
        .finish()
        .map_err(|e| ExportError::Write(e.to_string()))?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use vellum_format::Fence;

    use super::*;
    use crate::ports::{IssueChip, LiveData, LiveError, PageRef, Snapshot};

    struct Live;

    #[async_trait]
    impl LiveData for Live {
        async fn collection(
            &self,
            _fence: &Fence,
        ) -> Result<Snapshot, LiveError> {
            Ok(Snapshot {
                title: "Open risks".into(),
                columns: vec!["Risk".into(), "Owner".into()],
                rows: vec![vec!["Key rotation".into(), "Priya".into()]],
            })
        }
        async fn chart(&self, fence: &Fence) -> Result<Fence, LiveError> {
            Ok(Fence {
                literal: "labels: [W40, W41]\nvalues: [3, 5]\n".into(),
                args: String::new(),
                ..fence.clone()
            })
        }
        async fn issue(&self, _key: &str) -> Option<IssueChip> {
            None
        }
        async fn page(&self, _target: &str) -> Option<PageRef> {
            None
        }
        async fn person(&self, _handle: &str) -> Option<String> {
            None
        }
    }

    #[tokio::test]
    async fn live_blocks_are_frozen() {
        let doc = vellum_format::parse(
            "# Risks\n\n```collection\nid: risks\n```\n\n```chart\n\
                source: risks\ntitle: Trend\n```\n",
        )
        .document;
        let ports = Ports {
            live: std::sync::Arc::new(Live),
            ..Ports::default()
        };
        let out = markdown(&doc, &ports, "2026-10-29").await;
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert!(
            out.text.contains(r"*Open risks (as of 2026\-10-29)*"),
            "{}",
            out.text
        );
        assert!(
            out.text.contains("| Key rotation | Priya |"),
            "{}",
            out.text
        );
        assert!(out.text.contains("values: [3, 5]"), "{}", out.text);
        assert!(!out.text.contains("source: risks"), "{}", out.text);
    }

    #[tokio::test]
    async fn unavailable_live_data_keeps_the_source() {
        let doc =
            vellum_format::parse("```collection\nid: risks\n```\n").document;
        let out = markdown(&doc, &Ports::default(), "2026-10-29").await;
        assert_eq!(out.warnings.len(), 1);
        assert!(out.text.contains("```collection"));
    }

    #[test]
    fn zips_are_deterministic_and_reject_escapes() {
        let files = vec![
            ("Gateway.md".to_owned(), b"# Gateway\n".to_vec()),
            ("assets/a.png".to_owned(), vec![1, 2, 3]),
        ];
        assert_eq!(zip(&files).unwrap(), zip(&files).unwrap());
        for bad in ["../x.md", "/x.md", "a//b.md", ""] {
            assert!(zip(&[(bad.to_owned(), Vec::new())]).is_err(), "{bad}");
        }
        let repeated = vec![
            ("a.md".to_owned(), Vec::new()),
            ("a.md".to_owned(), Vec::new()),
        ];
        assert!(zip(&repeated).is_err());
    }
}
