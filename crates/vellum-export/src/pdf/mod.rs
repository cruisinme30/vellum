// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The print profile: a [`Flow`] typeset to PDF with Typst.
//!
//! The flow is handed to a fixed Typst template (`print.typ`) as JSON
//! data, never as generated markup, so document text cannot inject Typst
//! code. The typesetter sees a closed world: the template, the flow, the
//! theme and the flow's assets. It cannot read files, fetch packages or
//! reach the network.
//!
//! Charts and diagrams arrive as SVG and stay vectors in the PDF; headings
//! become PDF bookmarks; cross-references are links followed by the page
//! number Typst resolves; maths is typeset from the converted formula.
//!
//! Build one [`PdfEngine`] per process: loading the fonts is the slow
//! part, and the engine is shareable across threads.

mod themed;

use std::collections::HashMap;
use std::time::Instant;

use serde::Serialize;
use typst::diag::{FileError, FileResult, SourceDiagnostic};
use typst::foundations::{Bytes, Datetime, Smart};
use typst::layout::{Frame, FrameItem, Point};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt as _, World};
use typst_layout::PagedDocument;

use crate::error::ExportError;
use crate::flow::{Flow, Tone};
use crate::metrics;

/// The template every print export uses.
const TEMPLATE: &str = include_str!("print.typ");

/// The template that typesets one formula on a page that fits it, for
/// targets that show maths as an image.
const MATH_TEMPLATE: &str = concat!(
    "#set page(width: auto, height: auto, margin: 3pt)\n",
    "#set text(size: 13pt)\n",
    "#math.equation(block: false, eval(json(\"/flow.json\").math, mode: \
        \"math\"))\n",
);

/// Paper sizes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum Paper {
    /// ISO A4, 210 × 297 mm.
    #[default]
    #[serde(rename = "a4")]
    A4,
    /// US Letter, 8.5 × 11 in.
    #[serde(rename = "us-letter")]
    Letter,
}

/// The print theme: page, fonts and colours. Export themes are separate
/// from the app theme (R-TH-4); this is the built-in Light print theme.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrintTheme {
    /// Paper size.
    pub paper: Paper,
    /// Body text size in points.
    pub size: f64,
    /// Document language (ISO 639-1), for hyphenation and quotes.
    pub lang: String,
    /// Font families, most preferred first.
    pub fonts: Fonts,
    /// Colour roles as `#RRGGBB`.
    pub colors: Colors,
    /// Callout and chip colours per tone.
    pub tones: HashMap<Tone, ToneColors>,
}

/// Font families.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fonts {
    /// Body text.
    pub body: Vec<String>,
    /// Code.
    pub mono: Vec<String>,
}

/// Colour roles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Colors {
    /// Body text.
    pub text: String,
    /// Headings.
    pub heading: String,
    /// Secondary text.
    pub muted: String,
    /// Links and progress bars.
    pub primary: String,
    /// Rules and table lines.
    pub outline: String,
    /// Code and tile backgrounds.
    pub surface: String,
}

/// The colours and label of one tone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToneColors {
    /// Accent (border, icon, label).
    pub fg: String,
    /// Background.
    pub bg: String,
    /// Label printed on callouts.
    pub label: String,
}

impl Default for PrintTheme {
    fn default() -> Self {
        let tone = |t: Tone, fg: &str, bg: &str| {
            (
                t,
                ToneColors {
                    fg: fg.to_owned(),
                    bg: bg.to_owned(),
                    label: t.label().to_owned(),
                },
            )
        };
        Self {
            paper: Paper::A4,
            size: 10.5,
            lang: "en".to_owned(),
            fonts: Fonts {
                body: vec![
                    "Libertinus Serif".to_owned(),
                    "New Computer Modern".to_owned(),
                ],
                mono: vec!["DejaVu Sans Mono".to_owned()],
            },
            // Material 3 roles from the brief's seed (#4F5B92).
            colors: Colors {
                text: "#1D1B20".to_owned(),
                heading: "#1D1B20".to_owned(),
                muted: "#46464F".to_owned(),
                primary: "#4F5B92".to_owned(),
                outline: "#C5C6D0".to_owned(),
                surface: "#F0F2F9".to_owned(),
            },
            tones: [
                tone(Tone::Info, "#2F5DA8", "#E3ECFA"),
                tone(Tone::Warn, "#9A6200", "#FFF1D6"),
                tone(Tone::Good, "#2E7D32", "#E3F4E4"),
                tone(Tone::Crit, "#B3261E", "#FCE4E2"),
                tone(Tone::Tip, "#6750A4", "#EEE8FB"),
                tone(Tone::Note, "#5D5E67", "#EEEEF2"),
            ]
            .into_iter()
            .collect(),
        }
    }
}

/// A finished PDF.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfOutput {
    /// The PDF file.
    pub bytes: Vec<u8>,
    /// The text of each page, line by line, for search indexing and the
    /// golden corpus.
    pub pages: Vec<String>,
}

/// The Typst typesetter with its fonts loaded.
pub struct PdfEngine {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
}

impl std::fmt::Debug for PdfEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfEngine")
            .field("fonts", &self.fonts.len())
            .finish_non_exhaustive()
    }
}

impl Default for PdfEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfEngine {
    /// An engine with the bundled fonts (Libertinus Serif, New Computer
    /// Modern and its maths font, `DejaVu` Sans Mono).
    #[must_use]
    pub fn new() -> Self {
        Self::with_fonts(std::iter::empty())
    }

    /// An engine with the bundled fonts plus `extra` font files (TTF, OTF
    /// or collections), for themes that name other families.
    pub fn with_fonts(extra: impl IntoIterator<Item = Vec<u8>>) -> Self {
        let mut fonts: Vec<Font> = typst_assets::fonts()
            .flat_map(|data| Font::iter(Bytes::new(data)))
            .collect();
        for data in extra {
            fonts.extend(Font::iter(Bytes::new(data)));
        }
        Self {
            library: LazyHash::new(Library::default()),
            book: LazyHash::new(FontBook::from_fonts(&fonts)),
            fonts,
        }
    }

    /// Typesets `flow` with `theme`.
    ///
    /// # Errors
    ///
    /// [`ExportError::Typeset`] when the typesetter rejects the document
    /// (a bug: content never reaches Typst as code), and
    /// [`ExportError::Write`] when PDF writing fails.
    #[tracing::instrument(
        name = "export.pdf",
        skip_all,
        fields(blocks = flow.blocks.len(), assets = flow.assets.len(), pages)
    )]
    pub fn render(
        &self,
        flow: &Flow,
        theme: &PrintTheme,
    ) -> Result<PdfOutput, ExportError> {
        let started = Instant::now();
        let result = self.render_inner(flow, theme);
        if let Ok(output) = &result {
            let _ =
                tracing::Span::current().record("pages", output.pages.len());
        }
        metrics::export_done(
            "pdf",
            started,
            result.as_ref().map(|o| o.bytes.len()),
        );
        result
    }

    fn render_inner(
        &self,
        flow: &Flow,
        theme: &PrintTheme,
    ) -> Result<PdfOutput, ExportError> {
        let document = self.compile(flow, theme)?;
        let options = typst_pdf::PdfOptions {
            ident: Smart::Auto,
            ..typst_pdf::PdfOptions::default()
        };
        let bytes = typst_pdf::pdf(&document, &options).map_err(|errors| {
            ExportError::Write(
                errors
                    .iter()
                    .map(|e| e.message.to_string())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;
        let pages = document
            .pages()
            .iter()
            .map(|page| page_text(&page.frame))
            .collect();
        Ok(PdfOutput { bytes, pages })
    }
}

impl PdfEngine {
    /// Typesets `flow` like [`Self::render`] and draws its first
    /// `max_pages` pages as standalone SVG, for a preview the app shows as
    /// images (the app's CSP allows no frames or plugins). Returns the
    /// pages and the page count.
    ///
    /// # Errors
    ///
    /// As [`Self::render`].
    #[tracing::instrument(
        name = "export.pdf_preview",
        skip_all,
        fields(blocks = flow.blocks.len())
    )]
    pub fn preview(
        &self,
        flow: &Flow,
        theme: &PrintTheme,
        max_pages: usize,
    ) -> Result<(Vec<String>, usize), ExportError> {
        let started = Instant::now();
        let result = self.compile(flow, theme).map(|document| {
            let pages = document
                .pages()
                .iter()
                .take(max_pages)
                .map(|page| {
                    typst_svg::svg(
                        page,
                        &typst_svg::SvgOptions {
                            render_bleed: false,
                            pretty: false,
                        },
                    )
                })
                .collect::<Vec<_>>();
            (pages, document.pages().len())
        });
        metrics::export_done(
            "pdf_preview",
            started,
            result
                .as_ref()
                .map(|(pages, _)| pages.iter().map(String::len).sum()),
        );
        result
    }

    /// Lays `flow` out as pages.
    fn compile(
        &self,
        flow: &Flow,
        theme: &PrintTheme,
    ) -> Result<PagedDocument, ExportError> {
        let mut files = HashMap::new();
        let _ = files.insert(file_id("/flow.json")?, Bytes::new(json(flow)?));
        let _ = files.insert(file_id("/theme.json")?, Bytes::new(json(theme)?));
        for asset in &flow.assets {
            let _ = files.insert(
                file_id(&format!("/{}", asset.name))?,
                Bytes::new(asset.bytes.clone()),
            );
        }
        let main = file_id("/print.typ")?;
        let job = Job {
            engine: self,
            main,
            source: Source::new(main, TEMPLATE.to_owned()),
            files,
        };
        let warned = typst::compile::<PagedDocument>(&job);
        // Drop memoised results older than a few compilations so a long
        // running server keeps a flat memory profile.
        typst::comemo::evict(4);
        for warning in &warned.warnings {
            tracing::debug!(message = %warning.message, "typst warning");
        }
        warned.output.map_err(|errors| typeset_error(&errors))
    }

    /// Typesets one formula, given in Typst maths syntax (see
    /// [`crate::math::to_typst`]), as a standalone SVG.
    ///
    /// # Errors
    ///
    /// [`ExportError::Typeset`] when the formula does not typeset.
    pub fn math_svg(&self, typst_math: &str) -> Result<String, ExportError> {
        let mut files = HashMap::new();
        let data = serde_json::json!({ "math": typst_math });
        let _ = files.insert(file_id("/flow.json")?, Bytes::new(json(&data)?));
        let main = file_id("/math.typ")?;
        let job = Job {
            engine: self,
            main,
            source: Source::new(main, MATH_TEMPLATE.to_owned()),
            files,
        };
        let warned = typst::compile::<PagedDocument>(&job);
        typst::comemo::evict(4);
        let document =
            warned.output.map_err(|errors| typeset_error(&errors))?;
        let page = document.pages().first().ok_or_else(|| {
            ExportError::Typeset("the formula produced no page".into())
        })?;
        Ok(typst_svg::svg(
            page,
            &typst_svg::SvgOptions {
                render_bleed: false,
                pretty: false,
            },
        ))
    }
}

fn typeset_error(errors: &[SourceDiagnostic]) -> ExportError {
    ExportError::Typeset(
        errors
            .iter()
            .map(|e| {
                let hints: Vec<_> =
                    e.hints.iter().map(|h| h.v.to_string()).collect();
                if hints.is_empty() {
                    e.message.to_string()
                } else {
                    format!("{} ({})", e.message, hints.join("; "))
                }
            })
            .collect::<Vec<_>>()
            .join("; "),
    )
}

fn json<T: Serialize>(value: &T) -> Result<Vec<u8>, ExportError> {
    serde_json::to_vec(value).map_err(|e| {
        ExportError::Typeset(format!("cannot encode the flow: {e}"))
    })
}

/// Typst interns file IDs for the life of the process, so the names used
/// here come from a bounded set: three fixed files plus `a<n>.<ext>` for
/// `n` below [`crate::lower::MAX_ASSETS`].
fn file_id(path: &str) -> Result<FileId, ExportError> {
    let vpath = VirtualPath::new(path).map_err(|e| {
        ExportError::Typeset(format!("bad internal path {path}: {e:?}"))
    })?;
    Ok(RootedPath::new(VirtualRoot::Project, vpath).intern())
}

/// One compilation's view of the world.
struct Job<'a> {
    engine: &'a PdfEngine,
    main: FileId,
    source: Source,
    files: HashMap<FileId, Bytes>,
}

impl World for Job<'_> {
    fn library(&self) -> &LazyHash<Library> {
        &self.engine.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.engine.book
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main {
            Ok(self.source.clone())
        } else {
            Err(FileError::AccessDenied)
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.get(&id).cloned().ok_or(FileError::AccessDenied)
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.engine.fonts.get(index).cloned()
    }

    fn today(
        &self,
        _offset: Option<typst::foundations::Duration>,
    ) -> Option<Datetime> {
        None
    }
}

/// The text of a page, one line per baseline.
fn page_text(frame: &Frame) -> String {
    let mut runs = Vec::new();
    collect_runs(frame, Point::zero(), &mut runs);
    runs.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    let mut lines: Vec<Vec<Run>> = Vec::new();
    for run in runs {
        match lines.last_mut() {
            Some(line)
                if line
                    .first()
                    .is_some_and(|first| (first.y - run.y).abs() < 2.0) =>
            {
                line.push(run);
            }
            _ => lines.push(vec![run]),
        }
    }
    let mut out = String::new();
    for mut line in lines {
        line.sort_by(|a, b| a.x.total_cmp(&b.x));
        let mut end = f64::NEG_INFINITY;
        let mut text = String::new();
        for run in line {
            if !text.is_empty() && run.x > end + 1.5 && !text.ends_with(' ') {
                text.push(' ');
            }
            text.push_str(&run.text);
            end = run.x + run.width;
        }
        out.push_str(text.trim_end());
        out.push('\n');
    }
    out
}

struct Run {
    x: f64,
    y: f64,
    width: f64,
    text: String,
}

fn collect_runs(frame: &Frame, origin: Point, out: &mut Vec<Run>) {
    for (pos, item) in frame.items() {
        let at = origin + *pos;
        match item {
            FrameItem::Group(group) => {
                let shifted =
                    at + Point::new(group.transform.tx, group.transform.ty);
                collect_runs(&group.frame, shifted, out);
            }
            FrameItem::Text(text) => out.push(Run {
                x: at.x.to_pt(),
                y: at.y.to_pt(),
                width: text.width().to_pt(),
                text: text.text.to_string(),
            }),
            FrameItem::Shape(..)
            | FrameItem::Image(..)
            | FrameItem::Link(..)
            | FrameItem::Tag(_) => {}
        }
    }
}
