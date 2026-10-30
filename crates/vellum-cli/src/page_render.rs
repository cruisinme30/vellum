// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Server-side page rendering: `GET /v1/pages/{id}/render`.
//!
//! The drive reads the page for the reader (so only readers who may view it
//! get it), [`vellum_render::Renderer`] turns it into sanitised HTML, and
//! each `::: issues` block is answered by Work for that same reader through
//! [`WorkService::live_issues`]. Rendering lives in the binary because it
//! joins two services (the drive and Work), and services may only depend on
//! lower layers.
//!
//! Rendered pages are cached in memory by what they were rendered from
//! (ADR 0061, D269): the page ID, the renderer version and a hash of the
//! Markdown. A saved page has a new hash, so the cache never needs
//! invalidating, and every replica fills its own. Pages whose HTML depends
//! on the reader (live `::: issues` blocks) or on a diagram server that was
//! down (stale figures) are rendered every time. The drive still checks the
//! reader's permission and reads the file on every request; only the
//! render is skipped. The route revalidates, so a browser that has the page
//! gets `304 Not Modified` (D270).

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use vellum_contracts::{BoxFuture, HttpMethod, Operation, RequestContext, Scope, Service};
use vellum_drive::Drive;
use vellum_kernel::{Actor, Error, ErrorKind, ObjectId, UserId};
use vellum_render::figure::{FigureConfig, Figures};
use vellum_render::resolve::{IssueView, IssuesAt, NoResolver, Resolver};
use vellum_render::{RENDER_VERSION, RenderOptions, Renderer};
use vellum_work::UserRef;
use vellum_work_service::{LiveIssues, WorkService};

/// Finds a user's Work handle, for `@me` in issue filters.
pub type Handles = Arc<dyn Fn(UserId) -> BoxFuture<'static, Option<UserRef>> + Send + Sync>;

/// Names a page.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageKey {
    /// The page.
    pub id: ObjectId,
}

/// One heading of the page outline.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct OutlineEntry {
    /// 1 to 6.
    pub level: u8,
    /// The element id, for `#fragment` links.
    pub id: String,
    /// The heading text.
    pub text: String,
}

/// Something on the page that rendered with a fallback.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RenderWarning {
    /// The top-level block it came from, from 0.
    pub block: usize,
    /// What happened, for the author.
    pub message: String,
}

/// A page rendered for the reader.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RenderedPageView {
    /// The page.
    pub id: ObjectId,
    /// The front matter title, or the first level-1 heading.
    pub title: Option<String>,
    /// Sanitised HTML: one `<article class="vl-doc">`, styled by the theme
    /// stylesheet's `--vl-*` variables.
    pub html: String,
    /// The page outline.
    pub outline: Vec<OutlineEntry>,
    /// Blocks that rendered with a fallback.
    pub warnings: Vec<RenderWarning>,
    /// Diagrams shown from their last good picture because the diagram
    /// server was unavailable.
    pub stale_figures: usize,
    /// A strong `ETag` of the HTML (`sha256-…`).
    pub etag: String,
    /// The renderer version that produced it.
    pub render_version: u32,
}

/// One fenced block to draw, as the editor holds it (ADR 0068, D286).
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlockInput {
    /// The fence language, such as `mermaid` or `roadmap`.
    pub lang: String,
    /// The rest of the fence's opening line, such as `title="Launch"`.
    #[serde(default)]
    pub args: String,
    /// The fence body.
    pub source: String,
}

/// One block drawn for the editor.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RenderedBlockView {
    /// Sanitised HTML: one `<article class="vl-doc">` holding the block.
    pub html: String,
    /// What fell back, for the author; empty when the block drew cleanly.
    pub warnings: Vec<String>,
    /// Whether a diagram shows its last good picture because the diagram
    /// server was unavailable.
    pub stale: bool,
    /// The renderer version that produced it.
    pub render_version: u32,
}

/// Which accent colour to draw figures in (ADR 0068, D302).
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StylesheetInput {
    /// The app's accent colour as six hex digits, `rrggbb` (a leading `#`
    /// is accepted). Absent draws in Vellum's own colours.
    #[serde(default)]
    pub seed: Option<String>,
}

/// The app stylesheet for `seed`, or why the seed is unreadable.
///
/// # Errors
///
/// [`ErrorKind::InvalidInput`] when the seed is not six hex digits.
pub fn app_stylesheet(seed: Option<&str>) -> Result<String, Error> {
    let Some(raw) = seed.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(vellum_render::theme::scoped_stylesheet(
            RENDER_SCOPE,
            "data-theme",
        ));
    };
    let hex = raw.strip_prefix('#').unwrap_or(raw);
    let color: vellum_render::theme::Color = format!("#{hex}").parse().map_err(|_| {
        Error::new(
            ErrorKind::InvalidInput,
            "seed must be six hex digits, rrggbb",
        )
    })?;
    Ok(vellum_render::theme::scoped_stylesheet_from_seed(
        RENDER_SCOPE,
        "data-theme",
        color,
    ))
}

/// The stylesheet for rendered HTML inside the app.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RenderStylesheet {
    /// CSS scoped to [`RENDER_SCOPE`]; the theme follows the app's
    /// `<html data-theme>`.
    pub css: String,
    /// The class the app puts on the element that holds rendered HTML.
    pub scope: String,
    /// The renderer version it matches.
    pub render_version: u32,
}

/// The class of the element that holds rendered HTML in the web app.
pub const RENDER_SCOPE: &str = ".vl-render";
/// The longest block body the editor may ask to draw.
pub const MAX_BLOCK_SOURCE: usize = 64 * 1024;
/// The longest fence argument line.
const MAX_BLOCK_ARGS: usize = 512;

/// Checks a block and writes it as a Markdown fence. The fence is longer
/// than any backtick run in the body, so the body cannot close it early.
fn block_markdown(input: &BlockInput) -> Result<String, Error> {
    let invalid = |m: &str| Error::new(ErrorKind::InvalidInput, m.to_owned());
    let lang_ok = !input.lang.is_empty()
        && input.lang.len() <= 32
        && input
            .lang
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if !lang_ok {
        return Err(invalid("`lang` must be a fence language such as `mermaid`"));
    }
    if input.args.len() > MAX_BLOCK_ARGS || input.args.contains(['\n', '\r', '`']) {
        return Err(invalid("`args` must be one short line without backticks"));
    }
    if input.source.len() > MAX_BLOCK_SOURCE {
        return Err(invalid(
            "the block is too large to preview (64 KiB at most)",
        ));
    }
    let mut longest = 0;
    let mut run = 0;
    for c in input.source.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest.max(2) + 1);
    let args = input.args.trim();
    let head = if args.is_empty() {
        format!("{fence}{}", input.lang)
    } else {
        format!("{fence}{} {args}", input.lang)
    };
    let body = input.source.trim_end_matches(['\n', '\r']);
    Ok(format!("{head}\n{body}\n{fence}\n"))
}

/// The reader's view of the page's references.
struct PageResolver {
    issues: LiveIssues,
    /// Set when the page asked for anything that depends on the reader.
    /// Any resolver method added here that answers per reader must set it,
    /// or one reader's view would be cached for everyone.
    personal: AtomicBool,
}

impl Resolver for PageResolver {
    fn issues(&self, at: IssuesAt<'_>) -> Option<Result<IssueView, String>> {
        self.personal.store(true, Ordering::Relaxed);
        self.issues.view(at)
    }
}

/// Rendered pages kept per replica.
pub const RENDER_CACHE_PAGES: usize = 2048;
/// Pages with more HTML than this are not kept.
const MAX_CACHED_HTML: usize = 512 * 1024;
/// The most HTML a replica keeps, across all pages.
pub const RENDER_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// What the cache is keyed by: the page, the renderer and the Markdown.
type RenderKey = [u8; 32];

fn render_key(id: ObjectId, markdown: &str) -> RenderKey {
    let mut hash = Sha256::new();
    hash.update(id.to_string().as_bytes());
    hash.update(RENDER_VERSION.to_le_bytes());
    hash.update(markdown.as_bytes());
    hash.finalize().into()
}

/// The pages and the HTML bytes they hold.
struct Pages {
    lru: lru::LruCache<RenderKey, Arc<RenderedPageView>>,
    bytes: usize,
}

/// A least-recently-used cache of rendered pages, bounded by page count
/// and by total HTML.
struct RenderCache {
    pages: Mutex<Pages>,
    max_bytes: usize,
}

impl RenderCache {
    fn new(capacity: usize, max_bytes: usize) -> Self {
        let capacity = NonZeroUsize::new(capacity).unwrap_or(NonZeroUsize::MIN);
        Self {
            pages: Mutex::new(Pages {
                lru: lru::LruCache::new(capacity),
                bytes: 0,
            }),
            max_bytes,
        }
    }

    fn get(&self, key: &RenderKey) -> Option<Arc<RenderedPageView>> {
        self.pages.lock().ok()?.lru.get(key).cloned()
    }

    fn put(&self, key: RenderKey, page: Arc<RenderedPageView>) {
        let Ok(mut pages) = self.pages.lock() else {
            return;
        };
        let size = page.html.len();
        if let Some((_, old)) = pages.lru.push(key, page) {
            pages.bytes = pages.bytes.saturating_sub(old.html.len());
        }
        pages.bytes = pages.bytes.saturating_add(size);
        while pages.bytes > self.max_bytes {
            let Some((_, old)) = pages.lru.pop_lru() else {
                break;
            };
            pages.bytes = pages.bytes.saturating_sub(old.html.len());
        }
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.pages.lock().map_or(0, |p| p.lru.len())
    }
}

/// Renders pages for readers.
pub struct PageRenderService {
    drive: Arc<Drive>,
    work: Arc<WorkService>,
    handles: Handles,
    renderer: Renderer,
    cache: RenderCache,
}

impl std::fmt::Debug for PageRenderService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageRenderService").finish_non_exhaustive()
    }
}

impl PageRenderService {
    /// A service over the drive and Work, drawing diagrams with `figures`.
    #[must_use]
    pub fn new(
        drive: Arc<Drive>,
        work: Arc<WorkService>,
        handles: Handles,
        figures: Figures,
    ) -> Self {
        Self {
            drive,
            work,
            handles,
            renderer: Renderer::new(figures),
            cache: RenderCache::new(RENDER_CACHE_PAGES, RENDER_CACHE_BYTES),
        }
    }

    /// The figure settings from the environment (`VELLUM_KROKI_URL`).
    ///
    /// # Errors
    ///
    /// A malformed diagram-server setting.
    pub fn figures_from_env() -> Result<Figures, Error> {
        Figures::new(FigureConfig::from_env())
            .map_err(|e| Error::new(ErrorKind::InvalidInput, format!("diagram settings: {e}")))
    }

    /// Renders one page for the caller.
    ///
    /// # Errors
    ///
    /// `NotFound` or `PermissionDenied` from the drive, `InvalidInput` for
    /// a page that is not UTF-8 text.
    #[tracing::instrument(name = "render.page", skip_all, fields(cache))]
    pub async fn render(
        &self,
        ctx: &RequestContext,
        input: PageKey,
    ) -> Result<RenderedPageView, Error> {
        let user = ctx.principal.require_user()?;
        let (node, blob) = self.drive.read(&Actor::User { user }, input.id).await?;
        let markdown = String::from_utf8(blob.bytes.to_vec())
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "the page is not text"))?;
        let key = render_key(node.id, &markdown);
        if let Some(page) = self.cache.get(&key) {
            let _ = tracing::Span::current().record("cache", "hit");
            return Ok(RenderedPageView::clone(&page));
        }
        let _ = tracing::Span::current().record("cache", "miss");
        let doc = vellum_format::parse(&markdown).document;
        // Without a Work handle the reader has no issues to see, so every
        // `::: issues` block shows its placeholder.
        let issues = match (self.handles)(user).await {
            Some(viewer) => self.work.live_issues(&markdown, &viewer).await,
            None => LiveIssues::default(),
        };
        let options = RenderOptions {
            doc_key: Some(node.id.to_string()),
        };
        let resolver = PageResolver {
            issues,
            personal: AtomicBool::new(false),
        };
        let page = self.renderer.render(&doc, &resolver, &options).await;
        let blob = page.to_blob();
        let view = RenderedPageView {
            id: node.id,
            title: page.title,
            html: page.html,
            outline: page
                .toc
                .into_iter()
                .map(|e| OutlineEntry {
                    level: e.level,
                    id: e.id,
                    text: e.text,
                })
                .collect(),
            warnings: page
                .warnings
                .into_iter()
                .map(|w| RenderWarning {
                    block: w.block,
                    message: w.message,
                })
                .collect(),
            stale_figures: page.stale_figures,
            etag: blob.etag,
            render_version: RENDER_VERSION,
        };
        let shared = !resolver.personal.load(Ordering::Relaxed)
            && view.stale_figures == 0
            && view.html.len() <= MAX_CACHED_HTML;
        if shared {
            self.cache.put(key, Arc::new(view.clone()));
        }
        Ok(view)
    }
}

impl PageRenderService {
    /// Draws one block for the editor. It reads nothing on the server:
    /// links stay unresolved and `::: issues` shows its placeholder, so the
    /// answer depends only on the block, never on who asks.
    ///
    /// # Errors
    ///
    /// `InvalidInput` for a bad language, argument line or an oversized body.
    pub async fn render_block(
        &self,
        ctx: &RequestContext,
        input: BlockInput,
    ) -> Result<RenderedBlockView, Error> {
        let _user = ctx.principal.require_user()?;
        let markdown = block_markdown(&input)?;
        let doc = vellum_format::parse(&markdown).document;
        let page = self
            .renderer
            .render(&doc, &NoResolver, &RenderOptions::default())
            .await;
        Ok(RenderedBlockView {
            html: page.html,
            warnings: page.warnings.into_iter().map(|w| w.message).collect(),
            stale: page.stale_figures > 0,
            render_version: RENDER_VERSION,
        })
    }
}

impl Service for PageRenderService {
    fn name(&self) -> &'static str {
        "render"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let s = Arc::clone(&self);
        vec![
            Operation::query(
                "render.page",
                "/v1/pages/{id}/render",
                Scope::Read,
                move |ctx, input: PageKey| {
                    let s = Arc::clone(&s);
                    async move { s.render(&ctx, input).await }
                },
            )
            .summary("Renders a page as sanitised HTML for the reader")
            .revalidate()
            .description(
                "Reads the page with the caller's permissions and returns its HTML, \
                 outline and render warnings. Embedded `::: issues` blocks show the \
                 issues the caller may see.",
            ),
            {
                let s = Arc::clone(&self);
                Operation::mutation(
                    "render.block",
                    HttpMethod::Post,
                    "/v1/render/block",
                    Scope::Read,
                    move |ctx, input: BlockInput| {
                        let s = Arc::clone(&s);
                        async move { s.render_block(&ctx, input).await }
                    },
                )
                .summary("Draws one fenced block, such as a diagram, for the editor")
                .description(
                    "Renders a single fence (Mermaid, Kroki diagrams, charts, timelines, \
                     roadmaps, risk matrices, architecture, boards, calculators) to the \
                     same sanitised HTML as `render.page`. It changes nothing; it is a \
                     POST only because the body can be long.",
                )
            },
            Operation::query(
                "render.stylesheet",
                "/v1/render/stylesheet",
                Scope::Read,
                |_ctx, input: StylesheetInput| async move {
                    Ok::<_, Error>(RenderStylesheet {
                        css: app_stylesheet(input.seed.as_deref())?,
                        scope: RENDER_SCOPE.trim_start_matches('.').to_owned(),
                        render_version: RENDER_VERSION,
                    })
                },
            )
            .summary("The theme stylesheet for rendered HTML shown inside the app")
            .revalidate(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;
    use std::time::{Duration, Instant};

    use serde_json::{Value, json};
    use vellum_authz::{Authz, MemoryStore};
    use vellum_contracts::{CredentialKind, Principal, ScopeSet};
    use vellum_drive::SpaceKind;
    use vellum_drive::authz::AuthzAccess;
    use vellum_drive::memory::MemoryIndex;
    use vellum_kernel::{RequestId, Timestamp};
    use vellum_store::LocalStore;
    use vellum_work_service::{Directory, EventLog, StaticDirectory, SystemClock};

    use super::*;

    fn ctx(who: UserId) -> RequestContext {
        RequestContext {
            request_id: RequestId::generate(),
            principal: Principal::user(
                who,
                CredentialKind::Session,
                ScopeSet::FULL,
                Timestamp::now(),
            ),
            deadline: Instant::now() + Duration::from_secs(5),
            traceparent: None,
        }
    }

    async fn call(ops: &[Operation], who: UserId, op: &str, input: Value) -> Result<Value, Error> {
        let op = ops.iter().find(|o| o.id() == op).expect("operation");
        op.call(ctx(who), input).await
    }

    struct World {
        _dir: tempfile::TempDir,
        drive: Arc<Drive>,
        svc: Arc<PageRenderService>,
        ops: Vec<Operation>,
        mara: UserId,
        bob: UserId,
        me: Actor,
        space: vellum_kernel::SpaceId,
    }

    /// Mara's Engineering space, one ENG issue, and the render service.
    async fn world() -> World {
        let dir = tempfile::tempdir().unwrap();
        let store: Arc<dyn vellum_store::BlobStore> =
            Arc::new(LocalStore::open(dir.path()).await.unwrap());
        let index = Arc::new(MemoryIndex::new());
        let authz = Arc::new(Authz::new(MemoryStore::new()));
        let drive = Arc::new(Drive::new(
            Arc::clone(&store),
            index.clone(),
            Arc::new(AuthzAccess::new(authz, index)),
        ));
        let (mara, bob) = (UserId::generate(), UserId::generate());
        let directory =
            Arc::new(StaticDirectory::new().with(mara, UserRef::parse("mara").unwrap()));
        let (work, _) = WorkService::open(
            store,
            directory.clone(),
            Arc::new(EventLog::new()),
            Arc::new(SystemClock),
        )
        .await
        .unwrap();
        let work = Arc::new(work);
        let work_ops = Arc::clone(&work).operations();
        let _ = call(
            &work_ops,
            mara,
            "work.create_team",
            json!({"key": "ENG", "name": "Engineering", "lead": "mara"}),
        )
        .await
        .unwrap();
        let _ = call(
            &work_ops,
            mara,
            "work.create_issue",
            json!({"team": "ENG", "title": "Ship <login>", "state": "todo", "priority": 2}),
        )
        .await
        .unwrap();

        let me = Actor::User { user: mara };
        let space = drive
            .create_space(&me, SpaceKind::SharedDrive, "eng", "Engineering", None)
            .await
            .unwrap();
        let handles: Handles = Arc::new(move |user| {
            let directory = Arc::clone(&directory);
            Box::pin(async move { directory.handle(user).await })
        });
        let svc = Arc::new(PageRenderService::new(
            Arc::clone(&drive),
            work,
            handles,
            Figures::new(FigureConfig::default()).unwrap(),
        ));
        let ops = Arc::clone(&svc).operations();
        World {
            _dir: dir,
            drive,
            svc,
            ops,
            mara,
            bob,
            me,
            space: space.id,
        }
    }

    #[tokio::test]
    async fn pages_render_with_live_issues_for_the_reader_only() {
        let World {
            drive,
            svc,
            ops,
            mara,
            bob,
            me,
            space,
            ..
        } = world().await;
        let body = "# Launch plan\n\n::: issues\nfilter:\n  field: team\n  op: is\n  values: [ENG]\n:::\n\n::: issues\nfilter: [not, a, filter]\n:::\n";
        let page = drive
            .create_page(&me, space, None, "Launch plan", body)
            .await
            .unwrap();

        assert_eq!(ops[0].id(), "render.page");

        let out = call(&ops, mara, "render.page", json!({"id": page.id}))
            .await
            .unwrap();
        let html = out["html"].as_str().unwrap();
        assert!(html.contains("Ship &lt;login&gt;"), "{html}");
        assert!(html.contains("href=\"/i/ENG-1\""), "{html}");
        assert!(html.contains("High"), "{html}");
        assert_eq!(out["outline"][0]["text"], "Launch plan");
        assert!(out["etag"].as_str().unwrap().starts_with("sha256-"));
        assert_eq!(out["render_version"], RENDER_VERSION);
        let warnings = out["warnings"].as_array().unwrap();
        assert!(
            warnings
                .iter()
                .any(|w| w["message"].as_str().unwrap().contains("issues block 2")),
            "{warnings:?}"
        );

        // Someone without access to the page gets nothing from it.
        let denied = call(&ops, bob, "render.page", json!({"id": page.id}))
            .await
            .unwrap_err();
        assert_eq!(denied.kind(), ErrorKind::PermissionDenied);
        // Live issues depend on the reader, so that page is never cached.
        assert_eq!(svc.cache.len(), 0);
        assert!(ops[0].revalidates());
    }

    fn view(html: &str) -> Arc<RenderedPageView> {
        Arc::new(RenderedPageView {
            id: ObjectId::generate(),
            title: None,
            html: html.to_owned(),
            outline: Vec::new(),
            warnings: Vec::new(),
            stale_figures: 0,
            etag: String::new(),
            render_version: RENDER_VERSION,
        })
    }

    #[test]
    fn the_cache_keeps_to_its_byte_and_page_limits() {
        let cache = RenderCache::new(3, 10);
        cache.put([1; 32], view("aaaa"));
        cache.put([2; 32], view("bbbb"));
        assert_eq!(cache.len(), 2);
        // A third 4-byte page goes over 10 bytes: the oldest goes.
        cache.put([3; 32], view("cccc"));
        assert_eq!(cache.len(), 2);
        assert!(cache.get(&[1; 32]).is_none());
        // Replacing a page counts its new size, not both.
        cache.put([3; 32], view("cc"));
        cache.put([4; 32], view("dd"));
        assert_eq!(cache.len(), 3);
        // The page limit holds too.
        cache.put([5; 32], view("e"));
        assert_eq!(cache.len(), 3);
        assert!(cache.get(&[5; 32]).is_some());
    }

    #[tokio::test]
    async fn plain_pages_are_cached_until_they_are_saved() {
        let World {
            drive,
            svc,
            ops,
            mara,
            bob,
            me,
            space,
            ..
        } = world().await;
        // A plain page is rendered once, then served from the cache within
        // the cached-read budget (#74), until it is saved again.
        let mut body = String::from("# Northstar handbook\n\n");
        for n in 0..50 {
            let _ = write!(
                body,
                "## Section {n}\n\nSome **bold** text and a [link](https://northstar.dev).\n\n"
            );
        }
        let plain = drive
            .create_page(&me, space, None, "Handbook", &body)
            .await
            .unwrap();
        let first = call(&ops, mara, "render.page", json!({"id": plain.id}))
            .await
            .unwrap();
        assert_eq!(svc.cache.len(), 1);
        let started = Instant::now();
        let again = call(&ops, mara, "render.page", json!({"id": plain.id}))
            .await
            .unwrap();
        let cached = started.elapsed();
        assert_eq!(again, first);
        assert!(
            cached < Duration::from_millis(50),
            "cached read took {cached:?}"
        );
        assert_eq!(svc.cache.len(), 1);
        // The cache still asks the drive: Bob may not read it from there.
        let denied = call(&ops, bob, "render.page", json!({"id": plain.id}))
            .await
            .unwrap_err();
        assert_eq!(denied.kind(), ErrorKind::PermissionDenied);

        let current = drive.read(&me, plain.id).await.unwrap().1;
        let base = vellum_store::Base {
            bytes: current.bytes,
            etag: Some(current.meta.etag),
        };
        let _ = drive
            .save(
                &me,
                plain.id,
                base,
                bytes::Bytes::from_static(b"# Northstar handbook\n\nRewritten.\n"),
            )
            .await
            .unwrap();
        let saved = call(&ops, mara, "render.page", json!({"id": plain.id}))
            .await
            .unwrap();
        assert!(saved["html"].as_str().unwrap().contains("Rewritten."));
        assert_ne!(saved["etag"], first["etag"]);
    }

    #[test]
    fn blocks_become_one_fence_the_body_cannot_close() {
        let block = |lang: &str, args: &str, source: &str| BlockInput {
            lang: lang.into(),
            args: args.into(),
            source: source.into(),
        };
        assert_eq!(
            block_markdown(&block("mermaid", "", "graph TD\n  A --> B\n")).unwrap(),
            "```mermaid\ngraph TD\n  A --> B\n```\n"
        );
        assert_eq!(
            block_markdown(&block("roadmap", " title=\"Q3\" ", "x")).unwrap(),
            "```roadmap title=\"Q3\"\nx\n```\n"
        );
        // A body holding a fence gets a longer one.
        let tricky = block_markdown(&block("mermaid", "", "a\n````\n# not a heading")).unwrap();
        assert!(tricky.starts_with("`````mermaid\n"), "{tricky}");
        assert!(tricky.ends_with("\n`````\n"), "{tricky}");
        let doc = vellum_format::parse(&tricky).document;
        assert_eq!(doc.blocks.len(), 1);

        for bad in [
            block("", "", "x"),
            block("Mermaid", "", "x"),
            block("mermaid x", "", "x"),
            block("mermaid", "a\nb", "x"),
            block("mermaid", "`", "x"),
            block("mermaid", "", &"x".repeat(MAX_BLOCK_SOURCE + 1)),
        ] {
            let err = block_markdown(&bad).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }
    }

    #[tokio::test]
    async fn blocks_render_for_the_editor_without_reading_anything() {
        let w = world().await;
        let draw = |lang: &str, source: &str| {
            call(
                &w.ops,
                w.bob,
                "render.block",
                json!({"lang": lang, "source": source}),
            )
        };
        let timeline = draw("timeline", "2026-01-10 | Kickoff\n2026-03-01 | Beta\n")
            .await
            .unwrap();
        let html = timeline["html"].as_str().unwrap();
        assert!(html.contains("vl-timeline"), "{html}");
        assert!(html.contains("Kickoff"), "{html}");
        assert_eq!(timeline["warnings"], json!([]));
        assert_eq!(timeline["stale"], false);
        assert_eq!(timeline["render_version"], RENDER_VERSION);

        let mermaid = draw("mermaid", "graph TD\n  A[Plan] --> B[Ship]\n")
            .await
            .unwrap();
        assert!(mermaid["html"].as_str().unwrap().contains("<svg"));

        // A broken block still answers, with the warning.
        let broken = draw("calculator", "nothing: here\n").await.unwrap();
        assert!(
            !broken["warnings"].as_array().unwrap().is_empty(),
            "{broken}"
        );
        // With no diagram server, a Kroki diagram shows its source and why.
        let d2 = draw("d2", "gateway -> engine\n").await.unwrap();
        assert!(!d2["warnings"].as_array().unwrap().is_empty(), "{d2}");
        assert!(
            d2["html"]
                .as_str()
                .unwrap()
                .contains("gateway -&gt; engine"),
            "{d2}"
        );

        // Script never survives, whatever the block holds.
        let hostile = draw("board", "## Todo\n- <script>alert(1)</script>\n")
            .await
            .unwrap();
        assert!(!hostile["html"].as_str().unwrap().contains("<script"));

        // `::: issues` cannot be smuggled in to read Work as the caller.
        let smuggled = draw("mermaid", "x\n```\n\n::: issues\nfilter: {}\n:::\n")
            .await
            .unwrap();
        assert!(!smuggled["html"].as_str().unwrap().contains("vl-issue-row"));

        let err = draw("Mermaid", "x").await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn the_app_stylesheet_is_scoped_to_rendered_html() {
        let w = world().await;
        let out = call(&w.ops, w.mara, "render.stylesheet", json!({}))
            .await
            .unwrap();
        assert_eq!(out["scope"], "vl-render");
        let css = out["css"].as_str().unwrap();
        assert!(css.contains("[data-theme=\"sepia\"] .vl-render {"));
        assert!(css.contains("--vl-chart-1:"));
        let op = w
            .ops
            .iter()
            .find(|o| o.id() == "render.stylesheet")
            .unwrap();
        assert!(op.revalidates());
    }

    #[tokio::test]
    async fn the_app_stylesheet_follows_the_accent_colour() {
        let w = world().await;
        let plain = call(&w.ops, w.mara, "render.stylesheet", json!({}))
            .await
            .unwrap();
        let tinted = call(
            &w.ops,
            w.mara,
            "render.stylesheet",
            json!({"seed": FIXTURE_ACCENT}),
        )
        .await
        .unwrap();
        let hashed = call(
            &w.ops,
            w.mara,
            "render.stylesheet",
            json!({"seed": "#C2185B"}),
        )
        .await
        .unwrap();
        assert_ne!(tinted["css"], plain["css"]);
        assert_eq!(tinted["css"], hashed["css"]);
        let seed = "#c2185b".parse().unwrap();
        let dark =
            vellum_render::theme::Theme::from_seed(vellum_render::theme::BuiltinTheme::Dark, seed);
        let primary = format!("--vl-primary: {};", dark.get("primary"));
        assert!(tinted["css"].as_str().unwrap().contains(&primary));
        for bad in ["red", "12345", "#12345g", "1234567"] {
            let err = call(&w.ops, w.mara, "render.stylesheet", json!({"seed": bad}))
                .await
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "{bad}");
        }
    }

    /// Fences from the render corpus that the web app's UI tests draw: one
    /// of every block issue #522 lists. Images and collections need the
    /// server's files, so they stay out.
    fn fixture_fences() -> Vec<BlockInput> {
        let corpus = include_str!("../../../corpus/render/fences.md");
        let mut fences = Vec::new();
        let mut open: Option<BlockInput> = None;
        for line in corpus.lines() {
            match open.as_mut() {
                None => {
                    if let Some(head) = line.strip_prefix("```") {
                        let (lang, args) = head.split_once(' ').unwrap_or((head, ""));
                        open = Some(BlockInput {
                            lang: lang.to_owned(),
                            args: args.to_owned(),
                            source: String::new(),
                        });
                    }
                }
                Some(block) if line == "```" => {
                    let done = std::mem::take(block);
                    open = None;
                    if !matches!(done.lang.as_str(), "image" | "collection" | "sparkle") {
                        fences.push(done);
                    }
                }
                Some(block) => {
                    block.source.push_str(line);
                    block.source.push('\n');
                }
            }
        }
        fences
    }

    /// The accent colour the UI tests pick to see figures follow it.
    const FIXTURE_ACCENT: &str = "c2185b";

    /// The server's answers the web UI tests replay (web/e2e/support/
    /// render-fixture.gen.json): the app stylesheet, every fixture block
    /// drawn alone, and a page holding all of them. Fails when the renderer
    /// changes its output; `VELLUM_BLESS=1` rewrites the file.
    #[tokio::test]
    async fn the_web_render_fixture_matches_the_renderer() {
        let w = world().await;
        let fences = fixture_fences();
        assert!(fences.len() >= 11, "{}", fences.len());
        let mut blocks = serde_json::Map::new();
        let mut page =
            String::from("# Every block\n\nOne of every drawn block, from the server.\n");
        for fence in &fences {
            let out = w
                .svc
                .render_block(&ctx(w.mara), fence.clone())
                .await
                .unwrap();
            let _ = blocks.insert(fence.lang.clone(), serde_json::to_value(out).unwrap());
            let _ = write!(page, "\n{}", block_markdown(fence).unwrap());
        }
        let doc = vellum_format::parse(&page).document;
        let rendered = w
            .svc
            .renderer
            .render(&doc, &NoResolver, &RenderOptions::default())
            .await;
        let blob = rendered.to_blob();
        let stylesheet = call(&w.ops, w.mara, "render.stylesheet", json!({}))
            .await
            .unwrap();
        // The same sheet recoloured from an accent, for the D302 UI test.
        let accented = call(
            &w.ops,
            w.mara,
            "render.stylesheet",
            json!({"seed": FIXTURE_ACCENT}),
        )
        .await
        .unwrap();
        let fixture = json!({
            "markdown": page,
            "stylesheet": stylesheet,
            "accent": format!("#{FIXTURE_ACCENT}"),
            "stylesheet_accent": accented,
            "blocks": blocks,
            "page": {
                "id": "01M6400000PAGE00ROADMAP001",
                "title": rendered.title,
                "html": rendered.html,
                "outline": rendered.toc.iter().map(|e| json!({"level": e.level, "id": e.id, "text": e.text})).collect::<Vec<_>>(),
                "warnings": rendered.warnings.iter().map(|w| json!({"block": w.block, "message": w.message})).collect::<Vec<_>>(),
                "stale_figures": rendered.stale_figures,
                "etag": blob.etag,
                "render_version": RENDER_VERSION,
            },
        });
        let text = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../web/e2e/support/render-fixture.gen.json");
        if std::env::var_os("VELLUM_BLESS").is_some() {
            std::fs::write(&path, &text).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == text,
            "web/e2e/support/render-fixture.gen.json is out of date. Regenerate it with \
             `VELLUM_BLESS=1 cargo test -p vellum-cli web_render_fixture`."
        );
    }
}
