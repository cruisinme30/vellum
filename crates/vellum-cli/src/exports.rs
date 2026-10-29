// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Export and import over HTTP (D229–D231).
//!
//! Export, mounted at [`EXPORT_PREFIX`]:
//!
//! | Method | Path                                   | What                                  |
//! |--------|----------------------------------------|---------------------------------------|
//! | `GET`  | `/pages/{page_id}?format=…`            | a page as `pdf`, `docx`, `html`, `md` |
//! | `GET`  | `/pages/{page_id}?format=preview`      | the PDF's first pages as SVG (JSON)   |
//! | `GET`  | `/books/{book_id}?format=…&scope=…`    | a book, a section or a subtree        |
//! | `POST` | `/pages/{page_id}/google-docs`         | a page into the caller's Google Drive |
//!
//! Import, mounted at [`IMPORT_PREFIX`] with the upload body limit:
//!
//! | Method | Path                                         | What                         |
//! |--------|----------------------------------------------|------------------------------|
//! | `POST` | `?space=…&parent=…&source=…&name=…`          | a zip export or a Word file  |
//! | `POST` | `/google-doc`                                | a Google Doc, by file ID     |
//!
//! Export reads through the drive and the books service as the caller,
//! so it needs view rights and the `read` scope, and a book export
//! leaves out the sections the caller may not see. Import needs edit
//! rights on the target folder and `write:direct`, checked before the
//! body is read. Every imported file, uploaded or fetched from Google,
//! goes through [`upload::check`] (size, type, zip-bomb limits and the
//! virus scanner, failing closed) before an importer parses it, and lands
//! in a new folder (D230). Typesetting and importing are CPU-bound, so
//! they run on blocking threads behind a small number of slots (D231).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::sync::{Semaphore, SemaphorePermit};
use vellum_books_service::BooksService;
use vellum_books_service::api::{SectionKey, TocLine, TocNode};
use vellum_contracts::{Principal, RequestContext, Scope};
use vellum_drive::{Drive, NodeKind};
use vellum_export::book::{BookExport, BookItem, BookScope, BookSection};
use vellum_export::docx::{DocxOptions, PageSize};
use vellum_export::gdocs::{self, AccessToken};
use vellum_export::import::{self as importer, Source};
use vellum_export::pdf::{Paper, PdfEngine, PrintTheme};
use vellum_export::ports::{
    AssetError, AssetSource, FigureError, FigureRenderer, FigureSvg, NoLiveData, Ports,
};
use vellum_export::{ExportError, ExportWarning, Flow, ImportError, LowerOptions, Target};
use vellum_format::{Document, Fence};
use vellum_jobs::export::CollectionsEngine;
use vellum_kernel::{Actor, BookId, Error, ErrorKind, ObjectId, SpaceId};
use vellum_render::figure::{self, Figures};
use vellum_render::theme::{BuiltinTheme, Theme};
use vellum_security::upload::{self, Kind, Scanner, UploadError, UploadPolicy};

use crate::uploads::{
    UploadRefusal, caller, encode, invalid, json, parse_page, refused, request_id, respond,
};

/// Where the export routes are mounted.
pub const EXPORT_PREFIX: &str = "/api/v1/export";
/// Where the import routes are mounted.
pub const IMPORT_PREFIX: &str = "/api/v1/import";

/// Exports and imports that may typeset or parse at once (D231).
const SLOTS: usize = 2;
/// How long a request waits for a slot before it is told to retry.
const SLOT_WAIT: Duration = Duration::from_secs(30);
/// Pages a PDF preview draws.
const PREVIEW_PAGES: usize = 12;
/// Largest JSON body the Google routes read.
const MAX_JSON: usize = 64 * 1024;
/// How long Google Drive calls may take.
const GOOGLE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a book export may spend reading sections.
const BOOK_DEADLINE: Duration = Duration::from_secs(60);

/// What the routes share.
pub struct Transfer<S> {
    drive: Arc<Drive>,
    books: Option<Arc<BooksService>>,
    figures: Option<Arc<Figures>>,
    engine: Arc<PdfEngine>,
    google: Option<gdocs::Drive>,
    scanner: Option<Arc<S>>,
    policy: UploadPolicy,
    slots: Semaphore,
    base_url: Option<String>,
}

impl<S: Scanner + 'static> Transfer<S> {
    /// Export and import over `drive` and `books`, drawing figures with
    /// `figures` and scanning imports with `scanner`. Links to pages
    /// outside an export point at `base_url`.
    pub fn new(
        drive: Arc<Drive>,
        books: Option<Arc<BooksService>>,
        figures: Option<Arc<Figures>>,
        scanner: Option<Arc<S>>,
        policy: UploadPolicy,
        base_url: Option<String>,
    ) -> Self {
        let google = gdocs::Drive::new(GOOGLE_TIMEOUT)
            .inspect_err(|e| tracing::warn!(error = %e, "Google Docs export is off"))
            .ok();
        Self {
            drive,
            books,
            figures,
            engine: Arc::new(PdfEngine::new()),
            google,
            scanner,
            policy,
            slots: Semaphore::new(SLOTS),
            base_url,
        }
    }

    /// The export routes, for [`EXPORT_PREFIX`].
    pub fn export_router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/pages/{page_id}", get(export_page::<S>))
            .route("/pages/{page_id}/google-docs", post(export_google::<S>))
            .route("/books/{book_id}", get(export_book::<S>))
            .with_state(Arc::clone(self))
    }

    /// The import routes, for [`IMPORT_PREFIX`].
    pub fn import_router(self: &Arc<Self>) -> Router {
        Router::new()
            .route("/", post(import_upload::<S>))
            .route("/google-doc", post(import_google::<S>))
            .with_state(Arc::clone(self))
    }
}

impl<S> Transfer<S> {
    /// The ports one export by `actor` uses: figures from the renderer,
    /// images from the page's own files as `actor` may read them, and
    /// calculators from the collections engine.
    fn ports(&self, actor: &Actor) -> Ports {
        let figures: Arc<dyn FigureRenderer> = match &self.figures {
            Some(figures) => Arc::new(RenderFigures {
                figures: Arc::clone(figures),
                theme: Theme::builtin(BuiltinTheme::Light),
            }),
            None => Arc::new(vellum_export::ports::NoFigures),
        };
        Ports {
            figures,
            live: Arc::new(NoLiveData),
            assets: Arc::new(PageFiles {
                drive: Arc::clone(&self.drive),
                actor: actor.clone(),
            }),
            collections: Arc::new(CollectionsEngine),
        }
    }

    /// Waits for a typesetting or import slot.
    async fn slot(&self) -> Result<SemaphorePermit<'_>, Error> {
        match tokio::time::timeout(SLOT_WAIT, self.slots.acquire()).await {
            Ok(Ok(permit)) => Ok(permit),
            _ => Err(Error::new(
                ErrorKind::Unavailable,
                "the server is busy with other exports; try again shortly",
            )),
        }
    }

    fn options(&self, target: Target) -> LowerOptions {
        let mut options = LowerOptions::new(target, today());
        options.base_url.clone_from(&self.base_url);
        options
    }
}

/// The routes when `vellum serve` runs without a workspace: 503.
pub fn unavailable_router() -> Router {
    Router::new().fallback(|request: Request| async move {
        respond(
            request_id(&request),
            Error::new(
                ErrorKind::Unavailable,
                "export and import need a workspace index",
            ),
        )
    })
}

/// Draws figures with the render engine (#78).
struct RenderFigures {
    figures: Arc<Figures>,
    theme: Theme,
}

#[async_trait]
impl FigureRenderer for RenderFigures {
    async fn render(&self, fence: &Fence) -> Result<FigureSvg, FigureError> {
        match self.figures.render(fence, &self.theme).await {
            Ok(drawn) => Ok(FigureSvg {
                svg: drawn.svg,
                alt: drawn.alt,
                width: drawn.width,
                height: drawn.height,
            }),
            Err(figure::FigureError::Unsupported) => Err(FigureError::Unsupported),
            Err(figure::FigureError::Source { message }) => Err(FigureError::Source(message)),
            Err(figure::FigureError::Unavailable { message }) => {
                Err(FigureError::Unavailable(message))
            }
        }
    }
}

/// Loads `vellum://attachment/{page}/{name}` images as the exporter may
/// read them. Outside URLs are never fetched: they export as alt text.
struct PageFiles {
    drive: Arc<Drive>,
    actor: Actor,
}

#[async_trait]
impl AssetSource for PageFiles {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, AssetError> {
        let missing = || AssetError(format!("{url} is not a file of this workspace"));
        let rest = url
            .strip_prefix("vellum://attachment/")
            .ok_or_else(missing)?;
        let (page, name) = rest.split_once('/').ok_or_else(missing)?;
        let page: ObjectId = page.parse().map_err(|_| missing())?;
        let name = decode(name).ok_or_else(missing)?;
        let (_, bytes) = self
            .drive
            .attachment(&self.actor, page, &name)
            .await
            .map_err(|e| AssetError(e.to_string()))?;
        Ok(bytes.to_vec())
    }
}

/// The formats a page or book exports to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Pdf,
    Preview,
    Docx,
    Html,
    Markdown,
}

impl Format {
    fn parse(raw: Option<&str>) -> Result<Self, Error> {
        Ok(match raw.unwrap_or("pdf") {
            "pdf" => Self::Pdf,
            "preview" => Self::Preview,
            "docx" | "word" => Self::Docx,
            "html" => Self::Html,
            "md" | "markdown" => Self::Markdown,
            _ => {
                return Err(invalid("format must be pdf, preview, docx, html or md"));
            }
        })
    }

    const fn target(self) -> Target {
        match self {
            Self::Docx => Target::Office,
            _ => Target::Print,
        }
    }
}

/// The query of an export.
struct ExportQuery {
    format: Format,
    paper: Paper,
    scope: Option<String>,
    section: Option<String>,
}

impl ExportQuery {
    fn parse(request: &Request) -> Result<Self, Error> {
        let query = query_map(request);
        let paper = match query.get("paper").map(String::as_str) {
            None | Some("a4") => Paper::A4,
            Some("letter" | "us-letter") => Paper::Letter,
            Some(_) => return Err(invalid("paper must be a4 or letter")),
        };
        Ok(Self {
            format: Format::parse(query.get("format").map(String::as_str))?,
            paper,
            scope: query.get("scope").cloned(),
            section: query.get("section").cloned(),
        })
    }
}

/// A PDF preview: the first pages as SVG.
#[derive(Debug, Serialize)]
struct Preview {
    title: String,
    /// Standalone SVG documents, one per page.
    pages: Vec<String>,
    /// Pages in the whole PDF.
    page_count: usize,
    warnings: Vec<WarningView>,
}

#[derive(Debug, Serialize)]
struct WarningView {
    location: String,
    message: String,
}

impl From<ExportWarning> for WarningView {
    fn from(w: ExportWarning) -> Self {
        Self {
            location: w.location,
            message: w.message,
        }
    }
}

/// What to export, for the routes and for in-process callers (the MCP
/// `export` tool).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ExportItem {
    /// One page.
    Page(ObjectId),
    /// A book, or part of one.
    Book(BookId, BookScope),
}

/// A finished export file.
#[derive(Debug)]
pub(crate) struct Exported {
    /// `<title>.<ext>`.
    pub file_name: String,
    /// The media type.
    pub mime: &'static str,
    /// The file.
    pub bytes: Vec<u8>,
    /// Blocks that export differently from the screen.
    pub warnings: Vec<ExportWarning>,
}

/// An export in one format, or the PDF preview.
enum Made {
    File(Exported),
    Preview(Preview),
}

impl<S> Transfer<S> {
    /// Exports `item` as `principal` sees it: `format` is `pdf`, `docx`,
    /// `html` or `md` (pages only). Needs view rights and the `read`
    /// scope, like the routes.
    ///
    /// # Errors
    ///
    /// `unauthenticated` or `permission_denied` without the scope,
    /// `not_found` for what the caller cannot see, `invalid_input` for a
    /// bad format or scope, `unavailable` when the export slots are busy.
    #[allow(dead_code)] // the MCP `export` tool calls it
    pub(crate) async fn export(
        &self,
        principal: &Principal,
        request_id: vellum_kernel::RequestId,
        item: ExportItem,
        format: &str,
        paper: Paper,
    ) -> Result<Exported, Error> {
        let format = Format::parse(Some(format))?;
        if format == Format::Preview {
            return Err(invalid("format must be pdf, docx, html or md"));
        }
        match self
            .make(principal, request_id, item, format, paper)
            .await?
        {
            Made::File(file) => Ok(file),
            Made::Preview(_) => Err(Error::new(ErrorKind::Internal, "unexpected preview")),
        }
    }

    /// Reads, lowers and writes one export.
    async fn make(
        &self,
        principal: &Principal,
        request_id: vellum_kernel::RequestId,
        item: ExportItem,
        format: Format,
        paper: Paper,
    ) -> Result<Made, Error> {
        let user = principal.require_user()?;
        principal.require_scope(Scope::Read)?;
        let actor = Actor::User { user };
        let ports = self.ports(&actor);
        let lowered = match item {
            ExportItem::Page(page) => {
                let (title, doc) = read_page(&self.drive, &actor, page).await?;
                if format == Format::Markdown {
                    let _slot = self.slot().await?;
                    let out = vellum_export::bundle::markdown(&doc, &ports, &today()).await;
                    return Ok(Made::File(Exported {
                        file_name: format!("{}.md", file_stem(&title)),
                        mime: "text/markdown; charset=utf-8",
                        bytes: out.text.into_bytes(),
                        warnings: out.warnings,
                    }));
                }
                vellum_export::lower(&doc, &ports, &self.options(format.target())).await
            }
            ExportItem::Book(book, scope) => {
                if format == Format::Markdown {
                    return Err(invalid("books export as pdf, preview, docx or html"));
                }
                let books = self.books.as_ref().ok_or_else(|| {
                    Error::new(ErrorKind::Unavailable, "books are not served here")
                })?;
                let ctx = RequestContext {
                    request_id,
                    principal: principal.clone(),
                    deadline: Instant::now() + BOOK_DEADLINE,
                    traceparent: None,
                };
                let export = load_book(books, &ctx, book, scope).await?;
                vellum_export::book::lower_book(&export, &ports, &self.options(format.target()))
                    .await
            }
        };
        let _slot = self.slot().await?;
        self.write(format, paper, lowered.flow, lowered.warnings)
            .await
    }

    /// Writes a lowered flow in `format`, off the async workers.
    async fn write(
        &self,
        format: Format,
        paper: Paper,
        flow: Flow,
        warnings: Vec<ExportWarning>,
    ) -> Result<Made, Error> {
        let engine = Arc::clone(&self.engine);
        let theme = PrintTheme {
            paper,
            ..PrintTheme::default()
        };
        let docx = DocxOptions {
            page: match paper {
                Paper::Letter => PageSize::Letter,
                Paper::A4 => PageSize::A4,
            },
            ..DocxOptions::default()
        };
        let title = flow.title.clone();
        let made = tokio::task::spawn_blocking(move || -> Result<Written, ExportError> {
            Ok(match format {
                Format::Docx => Written::File(vellum_export::docx::write(&flow, &docx)?),
                Format::Html => {
                    Written::File(vellum_export::html::write(&flow, &theme).into_bytes())
                }
                Format::Preview => {
                    let (pages, count) = engine.preview(&flow, &theme, PREVIEW_PAGES)?;
                    Written::Preview(pages, count)
                }
                Format::Pdf | Format::Markdown => {
                    Written::File(engine.render(&flow, &theme)?.bytes)
                }
            })
        })
        .await
        .map_err(|e| Error::new(ErrorKind::Internal, format!("the export stopped: {e}")))?
        .map_err(export_error)?;
        tracing::info!(format = ?format, warnings = warnings.len(), "exported");
        Ok(match made {
            Written::Preview(pages, page_count) => Made::Preview(Preview {
                title,
                pages,
                page_count,
                warnings: warnings.into_iter().map(WarningView::from).collect(),
            }),
            Written::File(bytes) => {
                let (ext, mime) = match format {
                    Format::Docx => (
                        "docx",
                        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                    ),
                    Format::Html => ("html", "text/html; charset=utf-8"),
                    _ => ("pdf", "application/pdf"),
                };
                Made::File(Exported {
                    file_name: format!("{}.{ext}", file_stem(&title)),
                    mime,
                    bytes,
                    warnings,
                })
            }
        })
    }
}

enum Written {
    File(Vec<u8>),
    Preview(Vec<String>, usize),
}

/// An export as the routes send it: a download, or the preview as JSON.
fn send(made: Made) -> Response {
    match made {
        Made::Preview(preview) => json(StatusCode::OK, &preview),
        Made::File(file) => download(file),
    }
}

async fn export_page<S: Scanner + 'static>(
    State(transfer): State<Arc<Transfer<S>>>,
    Path(page_id): Path<String>,
    request: Request,
) -> Response {
    let id = request_id(&request);
    let result = async {
        let _ = caller(&request, Scope::Read)?;
        let query = ExportQuery::parse(&request)?;
        let principal = principal(&request);
        drop(request);
        let page = parse_page(&page_id)?;
        let made = transfer
            .make(
                &principal,
                id,
                ExportItem::Page(page),
                query.format,
                query.paper,
            )
            .await?;
        Ok(send(made))
    }
    .await;
    result.unwrap_or_else(|e| respond(id, e))
}

async fn export_book<S: Scanner + 'static>(
    State(transfer): State<Arc<Transfer<S>>>,
    Path(book_id): Path<String>,
    request: Request,
) -> Response {
    let id = request_id(&request);
    let result = async {
        let _ = caller(&request, Scope::Read)?;
        let query = ExportQuery::parse(&request)?;
        let principal = principal(&request);
        drop(request);
        let book: BookId = book_id
            .parse()
            .map_err(|_| Error::new(ErrorKind::NotFound, "book not found"))?;
        let scope = match (query.scope.as_deref(), query.section.clone()) {
            (None | Some("whole"), _) => BookScope::Whole,
            (Some("section"), Some(section)) => BookScope::Section(section),
            (Some("subtree"), Some(section)) => BookScope::Subtree(section),
            _ => {
                return Err(invalid(
                    "scope must be whole, or section or subtree with a section",
                ));
            }
        };
        let made = transfer
            .make(
                &principal,
                id,
                ExportItem::Book(book, scope),
                query.format,
                query.paper,
            )
            .await?;
        Ok(send(made))
    }
    .await;
    result.unwrap_or_else(|e| respond(id, e))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoogleExport {
    /// A Google `OAuth` access token with the `drive.file` scope.
    access_token: String,
    /// The Drive folder to create the document in.
    #[serde(default)]
    folder: Option<String>,
}

#[derive(Debug, Serialize)]
struct GoogleDoc {
    id: String,
    name: String,
    url: String,
}

async fn export_google<S: Scanner + 'static>(
    State(transfer): State<Arc<Transfer<S>>>,
    Path(page_id): Path<String>,
    request: Request,
) -> Response {
    let id = request_id(&request);
    let result = async {
        let actor = caller(&request, Scope::Read)?;
        let input: GoogleExport = read_json(request).await?;
        let google = transfer.google.as_ref().ok_or_else(|| {
            Error::new(
                ErrorKind::Unavailable,
                "Google Docs export is not available",
            )
        })?;
        let (title, doc) = read_page(&transfer.drive, &actor, parse_page(&page_id)?).await?;
        let ports = transfer.ports(&actor);
        let _slot = transfer.slot().await?;
        let lowered = vellum_export::lower(&doc, &ports, &transfer.options(Target::Office)).await;
        let file = google
            .export(
                &lowered.flow,
                &transfer.engine,
                &AccessToken::new(input.access_token),
                &title,
                input.folder.as_deref(),
            )
            .await
            .map_err(export_error)?;
        tracing::info!(warnings = lowered.warnings.len(), "exported to Google Docs");
        Ok(json(
            StatusCode::CREATED,
            &GoogleDoc {
                id: file.id,
                name: file.name,
                url: file.web_view_link,
            },
        ))
    }
    .await;
    result.unwrap_or_else(|e| respond(id, e))
}

/// What an import wrote.
#[derive(Debug, Serialize)]
struct ImportView {
    /// The new folder holding the import.
    folder_id: ObjectId,
    title: String,
    space_id: SpaceId,
    /// Pages, folders and collections created.
    nodes: usize,
    /// Files written.
    files: usize,
    warnings: Vec<WarningView>,
}

async fn import_upload<S: Scanner + 'static>(
    State(transfer): State<Arc<Transfer<S>>>,
    request: Request,
) -> Response {
    let id = request_id(&request);
    let result = async {
        let actor = caller(&request, Scope::WriteDirect)?;
        let query = query_map(&request);
        let source = parse_source(query.get("source").map(String::as_str))?;
        let (space, parent) = target(query.get("space"), query.get("parent"))?;
        let name = query
            .get("name")
            .filter(|n| !n.is_empty())
            .cloned()
            .ok_or_else(|| invalid("say the file's name in the `name` query parameter"))?;
        // Rights first, so a refused caller never sends the body.
        transfer
            .drive
            .may_import(&actor, space, parent)
            .await
            .map_err(Error::from)?;
        let length = request
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| invalid("an import must say its Content-Length"))?;
        if length > transfer.policy.max_bytes {
            return Err(refused(&UploadError::TooLarge(transfer.policy.max_bytes)));
        }
        let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
            .await
            .map_err(|_| invalid("the upload did not arrive whole"))?;
        let view = run_import(
            &transfer,
            &actor,
            source,
            space,
            parent,
            name,
            bytes.to_vec(),
        )
        .await?;
        Ok(json(StatusCode::CREATED, &view))
    }
    .await;
    result.unwrap_or_else(|e| respond(id, e))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoogleImport {
    /// A Google `OAuth` access token that may read the file.
    access_token: String,
    /// The Google Doc's file ID.
    file_id: String,
    /// Where the import goes.
    space_id: String,
    #[serde(default)]
    parent_id: Option<String>,
}

async fn import_google<S: Scanner + 'static>(
    State(transfer): State<Arc<Transfer<S>>>,
    request: Request,
) -> Response {
    let id = request_id(&request);
    let result = async {
        let actor = caller(&request, Scope::WriteDirect)?;
        let input: GoogleImport = read_json(request).await?;
        let (space, parent) = target(Some(&input.space_id), input.parent_id.as_ref())?;
        transfer
            .drive
            .may_import(&actor, space, parent)
            .await
            .map_err(Error::from)?;
        let google = transfer.google.as_ref().ok_or_else(|| {
            Error::new(
                ErrorKind::Unavailable,
                "Google Docs import is not available",
            )
        })?;
        let (name, bytes) = google
            .download_docx(&AccessToken::new(input.access_token), &input.file_id)
            .await
            .map_err(import_error)?;
        let name = if name.to_ascii_lowercase().ends_with(".docx") {
            name
        } else {
            format!("{name}.docx")
        };
        let view = run_import(&transfer, &actor, Source::Word, space, parent, name, bytes).await?;
        Ok(json(StatusCode::CREATED, &view))
    }
    .await;
    result.unwrap_or_else(|e| respond(id, e))
}

/// Checks the file, reads it with the importer and writes it to the drive.
async fn run_import<S: Scanner>(
    transfer: &Transfer<S>,
    actor: &Actor,
    source: Source,
    space: SpaceId,
    parent: Option<ObjectId>,
    name: String,
    bytes: Vec<u8>,
) -> Result<ImportView, Error> {
    let cleaned = upload::check(transfer.policy, transfer.scanner.as_deref(), name, bytes)
        .await
        .map_err(|e| refused(&e))?;
    if cleaned.kind != Kind::Zip {
        return Err(with_status(
            Error::new(
                ErrorKind::InvalidInput,
                "imports take a .zip export or a .docx file",
            ),
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ));
    }
    let title = folder_title(source, &cleaned.file_name);
    let file_name = cleaned.file_name.clone();
    let _slot = transfer.slot().await?;
    let set = tokio::task::spawn_blocking(move || {
        importer::import(source, &file_name, &cleaned.bytes, &CollectionsEngine)
    })
    .await
    .map_err(|e| Error::new(ErrorKind::Internal, format!("the import stopped: {e}")))?
    .map_err(import_error)?;
    let files = set
        .files
        .into_iter()
        .map(|f| (f.path, Bytes::from(f.bytes)))
        .collect();
    let imported = transfer
        .drive
        .import(actor, space, parent, &title, files)
        .await
        .map_err(Error::from)?;
    tracing::info!(source = source.name(), folder = %imported.folder.id, nodes = imported.nodes, "imported");
    Ok(ImportView {
        folder_id: imported.folder.id,
        title: imported.folder.title,
        space_id: imported.folder.space,
        nodes: imported.nodes,
        files: imported.files,
        warnings: set.warnings.into_iter().map(WarningView::from).collect(),
    })
}

/// Reads a page for `actor` and parses it.
async fn read_page(
    drive: &Drive,
    actor: &Actor,
    page: ObjectId,
) -> Result<(String, Document), Error> {
    let (node, blob) = drive.read(actor, page).await.map_err(Error::from)?;
    if node.kind != NodeKind::Page {
        return Err(invalid(
            "only pages export here; books have their own route",
        ));
    }
    let text = String::from_utf8_lossy(&blob.bytes);
    Ok((node.title, vellum_format::parse(&text).document))
}

/// The book as `ctx` sees it, with the sections `scope` needs loaded.
async fn load_book(
    books: &BooksService,
    ctx: &RequestContext,
    book: BookId,
    scope: BookScope,
) -> Result<BookExport, Error> {
    let info = books.get(ctx, book).await?;
    let mut ids = Vec::new();
    let mut stack: Vec<&TocNode> = info
        .toc
        .iter()
        .filter_map(|line| match line {
            TocLine::Chapter(node) => Some(node),
            _ => None,
        })
        .collect();
    while let Some(node) = stack.pop() {
        if let (Some(id), false) = (&node.id, node.draft) {
            ids.push(id.clone());
        }
        stack.extend(node.children.iter());
    }
    let mut docs = HashMap::with_capacity(ids.len());
    for section_id in ids {
        let doc = books
            .get_section(
                ctx,
                SectionKey {
                    book_id: book,
                    section_id: section_id.clone(),
                },
            )
            .await?;
        let _ = docs.insert(section_id, vellum_format::parse(&doc.source).document);
    }
    let items = info
        .toc
        .iter()
        .map(|line| match line {
            TocLine::Chapter(node) => BookItem::Section(Box::new(section(node, &mut docs))),
            TocLine::Part { title } => BookItem::Part(title.clone()),
            TocLine::Separator => BookItem::Separator,
        })
        .collect();
    Ok(BookExport {
        title: info.title,
        items,
        scope,
        toc_depth: info.max_depth,
    })
}

fn section(node: &TocNode, docs: &mut HashMap<String, Document>) -> BookSection {
    BookSection {
        id: node.id.clone(),
        title: node.title.clone(),
        number_label: node.number.clone(),
        label: node.label.clone(),
        depth: node.depth,
        draft: node.draft,
        document: node.id.as_ref().and_then(|id| docs.remove(id)),
        children: node.children.iter().map(|c| section(c, docs)).collect(),
    }
}

fn parse_source(raw: Option<&str>) -> Result<Source, Error> {
    Ok(match raw.unwrap_or_default() {
        "markdown" => Source::Markdown,
        "obsidian" => Source::Obsidian,
        "outline" => Source::Outline,
        "docmost" => Source::Docmost,
        "notion" => Source::Notion,
        "confluence" => Source::Confluence,
        "docx" | "word" => Source::Word,
        _ => {
            return Err(invalid(
                "source must be markdown, obsidian, outline, docmost, notion, confluence or docx",
            ));
        }
    })
}

fn target(
    space: Option<&String>,
    parent: Option<&String>,
) -> Result<(SpaceId, Option<ObjectId>), Error> {
    let space = space
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| invalid("say which space to import into with `space`"))?;
    let parent = match parent.filter(|p| !p.is_empty()) {
        Some(p) => Some(
            p.parse()
                .map_err(|_| Error::new(ErrorKind::NotFound, "folder not found"))?,
        ),
        None => None,
    };
    Ok((space, parent))
}

/// The new folder's name: the file's name without its extension, or the
/// source's name.
fn folder_title(source: Source, file_name: &str) -> String {
    let stem = file_name
        .rsplit_once('.')
        .map_or(file_name, |(stem, _)| stem)
        .trim();
    if stem.is_empty() {
        format!("{} import", source.name())
    } else {
        stem.to_owned()
    }
}

/// The signed-in caller as the gateway authenticated them.
fn principal(request: &Request) -> Principal {
    request
        .extensions()
        .get::<Principal>()
        .cloned()
        .unwrap_or_else(Principal::anonymous)
}

fn query_map(request: &Request) -> HashMap<String, String> {
    url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
        .into_owned()
        .collect()
}

async fn read_json<T: serde::de::DeserializeOwned>(request: Request) -> Result<T, Error> {
    let body = axum::body::to_bytes(request.into_body(), MAX_JSON)
        .await
        .map_err(|_| invalid("the body is missing or larger than 64 KiB"))?;
    serde_json::from_slice(&body)
        .map_err(|e| Error::new(ErrorKind::InvalidInput, format!("invalid body: {e}")))
}

/// An error sent with `status` instead of its kind's.
fn with_status(error: Error, status: StatusCode) -> Error {
    error.with_source(UploadRefusal(status))
}

fn export_error(error: ExportError) -> Error {
    match error {
        ExportError::Invalid(m) => Error::new(ErrorKind::InvalidInput, m),
        ExportError::TooLarge(m) => with_status(
            Error::new(ErrorKind::InvalidInput, format!("too large to export: {m}")),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        ExportError::Remote {
            service, message, ..
        } => with_status(
            Error::new(
                ErrorKind::Unavailable,
                format!("{service} failed: {message}"),
            ),
            StatusCode::BAD_GATEWAY,
        ),
        ExportError::NotConfigured(what) => {
            Error::new(ErrorKind::Unavailable, format!("{what} is not configured"))
        }
        other => {
            tracing::error!(error = %other, "export failed");
            Error::new(ErrorKind::Internal, "the export failed")
        }
    }
}

fn import_error(error: ImportError) -> Error {
    match error {
        ImportError::TooLarge(m) => with_status(
            Error::new(
                ErrorKind::InvalidInput,
                format!("import limit exceeded: {m}"),
            ),
            StatusCode::PAYLOAD_TOO_LARGE,
        ),
        ImportError::Remote {
            service, message, ..
        } => with_status(
            Error::new(
                ErrorKind::Unavailable,
                format!("{service} failed: {message}"),
            ),
            StatusCode::BAD_GATEWAY,
        ),
        other => with_status(
            Error::new(ErrorKind::InvalidInput, other.to_string()),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    }
}

/// A finished export as a download that a browser will not run.
fn download(file: Exported) -> Response {
    let mut response = (StatusCode::OK, Body::from(file.bytes)).into_response();
    let headers = response.headers_mut();
    let fixed = [
        (header::CONTENT_TYPE, file.mime),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; img-src data:; style-src 'unsafe-inline'; sandbox",
        ),
        (header::CACHE_CONTROL, "private, no-store"),
    ];
    for (name, value) in fixed {
        let _ = headers.insert(name, HeaderValue::from_static(value));
    }
    if let Ok(value) = HeaderValue::from_str(&format!(
        "attachment; filename*=UTF-8''{}",
        encode(&file.file_name)
    )) {
        let _ = headers.insert(header::CONTENT_DISPOSITION, value);
    }
    if let Ok(value) = HeaderValue::from_str(&file.warnings.len().to_string()) {
        let _ = headers.insert("vellum-export-warnings", value);
    }
    response
}

/// A file name stem from a title.
fn file_stem(title: &str) -> String {
    let stem: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '-'
            } else {
                c
            }
        })
        .take(120)
        .collect();
    let stem = stem.trim().trim_matches('.').to_owned();
    if stem.is_empty() {
        "export".to_owned()
    } else {
        stem
    }
}

/// Today's date, `YYYY-MM-DD`, which live blocks are frozen at.
fn today() -> String {
    jiff::Zoned::now().date().to_string()
}

/// Decodes `%XX` escapes; `None` for a malformed escape or bad UTF-8.
fn decode(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_stem("Q3 plan: v2/final"), "Q3 plan- v2-final");
        assert_eq!(file_stem("  ..  "), "export");
    }

    #[test]
    fn escapes_decode_or_refuse() {
        assert_eq!(decode("chart%20v2.png").as_deref(), Some("chart v2.png"));
        assert_eq!(decode("bad%2"), None);
        assert_eq!(decode("%FF"), None);
    }

    #[test]
    fn sources_and_formats_parse() {
        assert_eq!(parse_source(Some("notion")).ok(), Some(Source::Notion));
        assert!(parse_source(Some("evernote")).is_err());
        assert_eq!(Format::parse(None).ok(), Some(Format::Pdf));
        assert!(Format::parse(Some("exe")).is_err());
        assert_eq!(folder_title(Source::Notion, "Team wiki.zip"), "Team wiki");
        assert_eq!(folder_title(Source::Word, ".docx"), "docx import");
    }
}
