// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! What export needs from the rest of Vellum, as ports.
//!
//! Export never reads another service's tables. It asks these traits for
//! figures (the render engine draws charts and diagrams), live data (the
//! collections engine, Work and the page index answer with their current
//! state) and images (the blob store). Each has a `None…` implementation
//! that degrades to a readable fallback, so an export always succeeds.

use std::sync::Arc;

use async_trait::async_trait;
use vellum_format::Fence;
use vellum_format::blocks::Calculator;

/// A chart or diagram drawn as a standalone, sanitised SVG.
#[derive(Debug, Clone, PartialEq)]
pub struct FigureSvg {
    /// The SVG document. It has no scripts and no external references.
    pub svg: String,
    /// A text alternative for the figure.
    pub alt: String,
    /// Width in CSS pixels.
    pub width: f64,
    /// Height in CSS pixels.
    pub height: f64,
}

/// Why a figure could not be drawn.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FigureError {
    /// The renderer does not draw this fence; export uses its own fallback.
    #[error("not drawn by the renderer")]
    Unsupported,
    /// The fence source is invalid; export shows the source as code.
    #[error("invalid source: {0}")]
    Source(String),
    /// The renderer or its sidecar is down; export shows the source.
    #[error("renderer unavailable: {0}")]
    Unavailable(String),
}

/// Draws charts and diagrams. The render engine implements it.
#[async_trait]
pub trait FigureRenderer: Send + Sync {
    /// Draws `fence` (a static chart, timeline, roadmap, risk matrix,
    /// board, architecture or diagram fence) with the export theme.
    async fn render(&self, fence: &Fence) -> Result<FigureSvg, FigureError>;
}

/// A renderer that draws nothing, so every figure uses its fallback.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoFigures;

#[async_trait]
impl FigureRenderer for NoFigures {
    async fn render(&self, _fence: &Fence) -> Result<FigureSvg, FigureError> {
        Err(FigureError::Unsupported)
    }
}

/// The rows of a collection view at export time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// The view title.
    pub title: String,
    /// Column headers, in view order (hidden columns left out).
    pub columns: Vec<String>,
    /// Rows in view order, each with one display string per column.
    pub rows: Vec<Vec<String>>,
}

/// The state of an issue for a live chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueChip {
    /// The issue title.
    pub title: String,
    /// The status name, such as `In progress`.
    pub status: String,
}

/// A page another page links to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRef {
    /// The page ID, as in `vellum://doc/<id>`.
    pub id: String,
    /// The page title.
    pub title: String,
}

/// Why live data could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("live data unavailable: {0}")]
pub struct LiveError(pub String);

/// Current state for live blocks. Every call is made under the
/// permissions of the person exporting.
#[async_trait]
pub trait LiveData: Send + Sync {
    /// The rows of the view a `collection` fence embeds.
    async fn collection(&self, fence: &Fence) -> Result<Snapshot, LiveError>;

    /// A chart fence with `source=` replaced by static `labels:` and
    /// series lines holding the current values.
    async fn chart(&self, fence: &Fence) -> Result<Fence, LiveError>;

    /// The issue an issue key such as `ENG-142` names, if it exists.
    async fn issue(&self, key: &str) -> Option<IssueChip>;

    /// The page a wikilink target (a title or a `vellum://doc/` link)
    /// resolves to, if it exists and is visible.
    async fn page(&self, target: &str) -> Option<PageRef>;

    /// The display name of a mention handle, if it is a known person.
    async fn person(&self, handle: &str) -> Option<String>;
}

/// No live data: live blocks export with a note instead of rows.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoLiveData;

#[async_trait]
impl LiveData for NoLiveData {
    async fn collection(&self, _fence: &Fence) -> Result<Snapshot, LiveError> {
        Err(LiveError("no collection engine is connected".into()))
    }

    async fn chart(&self, _fence: &Fence) -> Result<Fence, LiveError> {
        Err(LiveError("no collection engine is connected".into()))
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

/// Why an image could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("image unavailable: {0}")]
pub struct AssetError(pub String);

/// Loads the bytes of images a page references. The implementation
/// decides what is reachable: workspace attachments through the blob
/// store, and outside URLs only through the SSRF-guarded fetcher.
#[async_trait]
pub trait AssetSource: Send + Sync {
    /// The bytes behind `url`, as written in the page.
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, AssetError>;
}

/// No images: every image exports as its alt text.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoAssets;

#[async_trait]
impl AssetSource for NoAssets {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, AssetError> {
        Err(AssetError(format!("no image source for {url}")))
    }
}

/// One calculator output, computed.
#[derive(Debug, Clone, PartialEq)]
pub struct CalcValue {
    /// The unit written after the formula, such as `$` or `days`.
    pub unit: Option<String>,
    /// The value, or why it could not be computed.
    pub value: Result<f64, String>,
}

/// A table read from an import source, to become a collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCollection {
    /// The collection title.
    pub title: String,
    /// The table as CSV, header row first.
    pub csv: String,
    /// Pages that hold row bodies, matched to rows by title.
    pub rows: Vec<RowPage>,
}

/// A page that holds the body of one row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowPage {
    /// The row title, as in the table's title column.
    pub title: String,
    /// The document ID the row takes.
    pub id: String,
    /// The row body as Vellum Markdown, without front matter.
    pub body: String,
}

/// A collection's files as `(path inside the collection folder, text)`,
/// and the problems found while building it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CollectionFiles {
    /// The files: schema, rows and views.
    pub files: Vec<(String, String)>,
    /// Rows or cells that could not be imported, one line each.
    pub problems: Vec<String>,
}

/// The collections engine, as export and import need it.
///
/// Calculators are computed with the collections formula language, and
/// imported databases become collections. Collections is a service on the
/// same layer as export, so it comes in through this port.
pub trait Collections: Send + Sync {
    /// Computes each output of `calc` from its stored inputs, in order;
    /// later outputs may use earlier ones. `as_of` (`YYYY-MM-DD`) answers
    /// `now()`.
    fn calculate(&self, calc: &Calculator, as_of: &str) -> Vec<CalcValue>;

    /// Builds the files of a new collection from an imported table.
    ///
    /// # Errors
    ///
    /// Returns why the table cannot become a collection; the import then
    /// keeps it as a CSV file and its row pages as ordinary pages.
    fn create(&self, table: &NewCollection) -> Result<CollectionFiles, String>;
}

/// No collections engine: calculator outputs show an error, and imported
/// databases stay CSV files with their row pages as ordinary pages.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCollections;

const NO_COLLECTIONS: &str = "no collections engine is connected";

impl Collections for NoCollections {
    fn calculate(&self, calc: &Calculator, _as_of: &str) -> Vec<CalcValue> {
        calc.outputs
            .iter()
            .map(|output| CalcValue {
                unit: output.unit.clone(),
                value: Err(NO_COLLECTIONS.into()),
            })
            .collect()
    }

    fn create(
        &self,
        _table: &NewCollection,
    ) -> Result<CollectionFiles, String> {
        Err(NO_COLLECTIONS.into())
    }
}

/// The ports one export uses.
#[derive(Clone)]
pub struct Ports {
    /// Draws charts and diagrams.
    pub figures: Arc<dyn FigureRenderer>,
    /// Answers live blocks.
    pub live: Arc<dyn LiveData>,
    /// Loads images.
    pub assets: Arc<dyn AssetSource>,
    /// Computes calculators.
    pub collections: Arc<dyn Collections>,
}

impl Default for Ports {
    fn default() -> Self {
        Self {
            figures: Arc::new(NoFigures),
            live: Arc::new(NoLiveData),
            assets: Arc::new(NoAssets),
            collections: Arc::new(NoCollections),
        }
    }
}

impl std::fmt::Debug for Ports {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ports").finish_non_exhaustive()
    }
}
