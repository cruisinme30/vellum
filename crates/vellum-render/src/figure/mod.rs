// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Figures: every fence drawn as SVG.
//!
//! [`Figures`] is shared by page rendering and by exports. Built-in figures
//! (charts, roadmaps, risk matrices, timelines, boards, architecture) and
//! Mermaid are drawn in-process; `D2`, PlantUML, Graphviz and Excalidraw go to
//! the optional Kroki sidecar, with the last good SVG served when it is
//! down; draw.io fences carry their own SVG export.

pub mod cache;
pub mod chart;
pub mod kroki;
pub mod mermaid;
pub mod plan;
pub mod svg;

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sha2::{Digest as _, Sha256};
use vellum_format::blocks::Data;
use vellum_format::{Fence, FenceKind};

use crate::model;
use crate::sanitize::xml::sanitize_island;
use crate::theme::Theme;
use cache::{MemoryCache, SvgCache};
use kroki::{Kroki, KrokiError};

/// Bump when figure output changes, so cached SVGs are not reused.
pub const FIGURE_VERSION: u32 = 1;

/// Settings for [`Figures`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FigureConfig {
    /// Kroki base URL; `None` disables `D2`, PlantUML, Graphviz and Excalidraw
    /// (their source is shown instead).
    pub kroki_url: Option<String>,
    /// Per-request Kroki timeout.
    pub timeout: Duration,
    /// Diagrams kept in the last-good-SVG cache.
    pub cache_capacity: usize,
    /// Time allowed for one Mermaid diagram.
    pub mermaid_deadline: Duration,
}

/// The environment variable that holds the Kroki base URL.
pub const KROKI_URL_ENV: &str = "VELLUM_KROKI_URL";

impl FigureConfig {
    /// The defaults, with the Kroki URL from `VELLUM_KROKI_URL` when it is
    /// set and not empty.
    #[must_use]
    pub fn from_env() -> Self {
        let kroki_url = std::env::var(KROKI_URL_ENV)
            .ok()
            .map(|u| u.trim().to_owned())
            .filter(|u| !u.is_empty());
        Self {
            kroki_url,
            ..Self::default()
        }
    }
}

impl Default for FigureConfig {
    fn default() -> Self {
        Self {
            kroki_url: None,
            timeout: Duration::from_secs(5),
            cache_capacity: 2_048,
            mermaid_deadline: Duration::from_secs(2),
        }
    }
}

/// A rendered figure.
#[derive(Debug, Clone, PartialEq)]
pub struct Figure {
    /// The SVG. From [`Figures::render`] it is sanitised and standalone;
    /// inside pages it still uses `var(--vl-…)` colours.
    pub svg: String,
    /// Plain-text description for alt text.
    pub alt: String,
    /// Width in CSS pixels.
    pub width: f64,
    /// Height in CSS pixels.
    pub height: f64,
    /// True when Kroki was unavailable and an older SVG of this diagram is
    /// shown.
    pub stale: bool,
}

/// Why a fence has no figure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FigureError {
    /// The fence is not drawn as a figure (code, tables, calculators…).
    #[error("this fence is not a figure")]
    Unsupported,
    /// The fence's content is invalid; show the source with this message.
    #[error("{message}")]
    Source {
        /// What is wrong, for the reader.
        message: String,
    },
    /// The diagram service is unavailable and nothing is cached.
    #[error("{message}")]
    Unavailable {
        /// What failed.
        message: String,
    },
}

impl FigureError {
    fn source(message: impl Into<String>) -> Self {
        Self::Source {
            message: message.into(),
        }
    }
}

/// How a fence is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// Drawn by this crate.
    Builtin,
    /// The bundled Mermaid engine.
    Mermaid,
    /// The Kroki sidecar, with this endpoint.
    Kroki(&'static str),
    /// The fence body is an SVG (draw.io editable SVG export).
    Embedded,
}

/// How `fence` is drawn, or `None` when it is not a figure.
#[must_use]
pub fn engine(fence: &Fence) -> Option<Engine> {
    match fence.kind() {
        FenceKind::Chart
        | FenceKind::Timeline
        | FenceKind::Roadmap
        | FenceKind::RiskMatrix
        | FenceKind::Board
        | FenceKind::Architecture => Some(Engine::Builtin),
        FenceKind::Diagram => match fence.lang.as_str() {
            "mermaid" => Some(Engine::Mermaid),
            "drawio" => Some(Engine::Embedded),
            other => kroki::endpoint(other).map(Engine::Kroki),
        },
        _ => None,
    }
}

/// A stable key for a fence's content.
#[must_use]
pub fn content_key(fence: &Fence) -> String {
    let mut h = Sha256::new();
    h.update(FIGURE_VERSION.to_le_bytes());
    for part in [&fence.lang, &fence.args, &fence.literal] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    let digest = h.finalize();
    digest
        .iter()
        .take(16)
        .fold(String::with_capacity(32), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Reads `width`/`height` (or the `viewBox`) of an SVG root.
fn svg_size(svg: &str) -> (f64, f64) {
    let head = svg
        .get(..svg.find('>').unwrap_or(svg.len()))
        .unwrap_or_default();
    let attr = |name: &str| -> Option<&str> {
        let i = head.find(&format!(" {name}=\""))? + name.len() + 3;
        let rest = head.get(i..)?;
        rest.get(..rest.find('"')?)
    };
    let px = |v: &str| {
        v.trim_end_matches("px")
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && *v > 0.0)
    };
    if let Some(vb) = attr("viewBox") {
        let n: Vec<f64> = vb
            .split([' ', ','])
            .filter_map(|v| v.parse().ok())
            .collect();
        if let [_, _, w, h] = n.as_slice() {
            return (*w, *h);
        }
    }
    (
        attr("width").and_then(px).unwrap_or(640.0),
        attr("height").and_then(px).unwrap_or(360.0),
    )
}

fn drawn(d: svg::Drawing) -> Figure {
    Figure {
        svg: d.svg,
        alt: d.alt,
        width: d.width,
        height: d.height,
        stale: false,
    }
}

/// Draws a built-in figure from the fence's typed data.
fn builtin(fence: &Fence) -> Result<Figure, FigureError> {
    let Some(read) = Data::from_fence(fence) else {
        return Err(FigureError::Unsupported);
    };
    let d = match read.data {
        Data::Chart(c) => chart::draw(&model::chart(&c).map_err(FigureError::source)?),
        Data::Roadmap(r) => plan::roadmap(&model::roadmap(&r).map_err(FigureError::source)?),
        Data::RiskMatrix(m) if m.risks.is_empty() => {
            return Err(FigureError::source("risk matrix has no risks"));
        }
        Data::RiskMatrix(m) => plan::risk_matrix(&m.risks),
        Data::Timeline(t) => plan::timeline(&t.items),
        Data::Board(b) => plan::board(&b.columns),
        Data::Architecture(a) if a.lanes.is_empty() => {
            return Err(FigureError::source("architecture has no lanes"));
        }
        Data::Architecture(a) => plan::architecture(&a),
        _ => return Err(FigureError::Unsupported),
    };
    Ok(drawn(d))
}

fn alt_for(fence: &Fence) -> String {
    let first = fence
        .literal
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    format!(
        "{} diagram: {}",
        fence.lang,
        first.chars().take(120).collect::<String>()
    )
}

struct Inner {
    config: FigureConfig,
    kroki: Option<Kroki>,
    cache: Arc<dyn SvgCache>,
}

/// Renders figures; cheap to clone and safe to share.
#[derive(Clone)]
pub struct Figures {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Figures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Figures")
            .field("config", &self.inner.config)
            .finish_non_exhaustive()
    }
}

impl Default for Figures {
    fn default() -> Self {
        Self::with_cache(
            FigureConfig::default(),
            Arc::new(MemoryCache::new(FigureConfig::default().cache_capacity)),
        )
        .unwrap_or_else(|_| Self {
            inner: Arc::new(Inner {
                config: FigureConfig::default(),
                kroki: None,
                cache: Arc::new(MemoryCache::new(1)),
            }),
        })
    }
}

impl Figures {
    /// Figures with an in-memory cache.
    ///
    /// # Errors
    ///
    /// Returns a message when the Kroki client cannot be built.
    pub fn new(config: FigureConfig) -> Result<Self, String> {
        let cache = Arc::new(MemoryCache::new(config.cache_capacity));
        Self::with_cache(config, cache)
    }

    /// Figures with a caller-supplied cache (for example one backed by the
    /// blob store, so the fallback survives restarts).
    ///
    /// # Errors
    ///
    /// Returns a message when the Kroki client cannot be built.
    pub fn with_cache(config: FigureConfig, cache: Arc<dyn SvgCache>) -> Result<Self, String> {
        let kroki = match &config.kroki_url {
            Some(url) => Some(Kroki::new(url, config.timeout)?),
            None => None,
        };
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                kroki,
                cache,
            }),
        })
    }

    /// Draws anything that needs no network: built-ins, Mermaid and draw.io.
    /// Returns `None` for Kroki diagrams.
    #[must_use]
    pub(crate) fn draw_local(&self, fence: &Fence) -> Option<Result<Figure, FigureError>> {
        let started = Instant::now();
        let result = match engine(fence)? {
            Engine::Builtin => builtin(fence),
            Engine::Mermaid => {
                let key = content_key(fence);
                match self.inner.cache.get(&key) {
                    Some(svg) => Ok(svg),
                    None => mermaid::render(&fence.literal, self.inner.config.mermaid_deadline)
                        .inspect(|svg| self.inner.cache.put(&key, None, svg))
                        .map_err(FigureError::source),
                }
                .map(|svg| {
                    let (width, height) = svg_size(&svg);
                    Figure {
                        alt: alt_for(fence),
                        svg,
                        width,
                        height,
                        stale: false,
                    }
                })
            }
            Engine::Embedded => {
                let body = fence.literal.trim();
                if body.starts_with("<svg") || body.starts_with("<?xml") {
                    let (width, height) = svg_size(body);
                    Ok(Figure {
                        svg: body.to_owned(),
                        alt: alt_for(fence),
                        width,
                        height,
                        stale: false,
                    })
                } else {
                    Err(FigureError::source(
                        "a drawio fence holds the diagram's editable SVG export",
                    ))
                }
            }
            Engine::Kroki(_) => return None,
        };
        tracing::debug!(
            histogram.vellum_render_figure_ms = started.elapsed().as_secs_f64() * 1e3,
            lang = fence.lang.as_str(),
            ok = result.is_ok(),
            "figure drawn"
        );
        Some(result)
    }

    /// The cached SVG for a Kroki diagram, by content and then by slot.
    pub(crate) fn kroki_cached(
        &self,
        fence: &Fence,
        slot: Option<&str>,
    ) -> Result<Figure, FigureError> {
        let key = content_key(fence);
        let (svg, stale) = match self.inner.cache.get(&key) {
            Some(svg) => (svg, false),
            None => match slot.and_then(|s| self.inner.cache.last(s)) {
                Some(svg) => (svg, true),
                None => {
                    let message = if self.inner.kroki.is_some() {
                        "the diagram service is unavailable"
                    } else {
                        "the diagram service (Kroki) is not configured"
                    };
                    return Err(FigureError::Unavailable {
                        message: message.to_owned(),
                    });
                }
            },
        };
        let (width, height) = svg_size(&svg);
        Ok(Figure {
            svg,
            alt: alt_for(fence),
            width,
            height,
            stale,
        })
    }

    /// Renders a Kroki diagram, falling back to the cache.
    pub(crate) async fn kroki(
        &self,
        fence: &Fence,
        slot: Option<&str>,
    ) -> Result<Figure, FigureError> {
        let Some(Engine::Kroki(endpoint)) = engine(fence) else {
            return Err(FigureError::Unsupported);
        };
        let key = content_key(fence);
        if let Some(svg) = self.inner.cache.get(&key) {
            if let Some(slot) = slot {
                self.inner.cache.put(&key, Some(slot), &svg);
            }
            let (width, height) = svg_size(&svg);
            return Ok(Figure {
                svg,
                alt: alt_for(fence),
                width,
                height,
                stale: false,
            });
        }
        let Some(client) = &self.inner.kroki else {
            return self.kroki_cached(fence, slot);
        };
        match client.render(endpoint, &fence.literal).await {
            Ok(svg) => {
                // Only well-formed, sanitisable SVG is cached.
                if let Err(e) = sanitize_island(&svg, "k") {
                    return Err(FigureError::source(format!(
                        "the diagram service returned invalid SVG: {e}"
                    )));
                }
                self.inner.cache.put(&key, slot, &svg);
                let (width, height) = svg_size(&svg);
                Ok(Figure {
                    svg,
                    alt: alt_for(fence),
                    width,
                    height,
                    stale: false,
                })
            }
            Err(KrokiError::Source(message)) => Err(FigureError::Source { message }),
            Err(KrokiError::Unavailable(message)) => {
                tracing::warn!(
                    error = message.as_str(),
                    "Kroki unavailable; using the last good SVG"
                );
                self.kroki_cached(fence, slot)
            }
        }
    }

    /// Renders a figure as standalone, sanitised SVG with the theme's
    /// colours baked in: the API exports use (PDF, Word, Google Docs).
    ///
    /// # Errors
    ///
    /// See [`FigureError`].
    pub async fn render(&self, fence: &Fence, theme: &Theme) -> Result<Figure, FigureError> {
        let figure = match self.draw_local(fence) {
            Some(result) => result?,
            None => self.kroki(fence, None).await?,
        };
        standalone(figure, fence, theme)
    }

    /// Like [`Self::render`] but never calls Kroki: Kroki diagrams come only
    /// from the cache.
    ///
    /// # Errors
    ///
    /// See [`FigureError`].
    pub fn render_cached(&self, fence: &Fence, theme: &Theme) -> Result<Figure, FigureError> {
        let figure = match self.draw_local(fence) {
            Some(result) => result?,
            None => self.kroki_cached(fence, None)?,
        };
        standalone(figure, fence, theme)
    }
}

fn standalone(mut figure: Figure, fence: &Fence, theme: &Theme) -> Result<Figure, FigureError> {
    let prefix = format!("v{}", content_key(fence).get(..8).unwrap_or("0"));
    let clean = sanitize_island(&figure.svg, &prefix)
        .map_err(|e| FigureError::source(format!("invalid SVG: {e}")))?;
    figure.svg = theme.resolve_vars(&clean);
    Ok(figure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::BuiltinTheme;
    use vellum_format::Attributes;

    fn fence(lang: &str, literal: &str) -> Fence {
        Fence {
            lang: lang.into(),
            args: String::new(),
            attrs: Attributes::default(),
            literal: literal.into(),
        }
    }

    #[tokio::test]
    async fn exports_standalone_svg_with_baked_colours() {
        let figures = Figures::default();
        let theme = Theme::builtin(BuiltinTheme::Light);
        let f = figures
            .render(
                &fence("chart", "title: T\ntype: bar\nlabels: [A]\nvalues: [1]\n"),
                &theme,
            )
            .await
            .unwrap();
        assert!(!f.svg.contains("var(--vl-"), "{}", f.svg);
        assert!(f.svg.contains(&theme.get("chart-1").to_string()));
        assert!(f.alt.starts_with("Bar chart: T."));

        let m = figures
            .render(&fence("mermaid", "flowchart LR\n  A --> B\n"), &theme)
            .await
            .unwrap();
        assert!(m.svg.starts_with("<svg"));
        assert!(m.width > 0.0);
    }

    #[tokio::test]
    async fn classifies_failures() {
        let figures = Figures::default();
        let theme = Theme::builtin(BuiltinTheme::Dark);
        assert_eq!(
            figures.render(&fence("rust", "fn x(){}"), &theme).await,
            Err(FigureError::Unsupported)
        );
        assert!(matches!(
            figures.render(&fence("chart", "title: x"), &theme).await,
            Err(FigureError::Source { .. })
        ));
        assert!(matches!(
            figures.render(&fence("d2", "a -> b"), &theme).await,
            Err(FigureError::Unavailable { .. })
        ));
        assert!(matches!(
            figures.render(&fence("drawio", "<mxfile/>"), &theme).await,
            Err(FigureError::Source { .. })
        ));
    }

    #[test]
    fn falls_back_to_the_last_svg_of_a_slot() {
        let figures = Figures::default();
        let old = fence("d2", "a -> b");
        figures.inner.cache.put(
            &content_key(&old),
            Some("doc#0"),
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 20\"/>",
        );
        let fresh = figures.kroki_cached(&old, Some("doc#0")).unwrap();
        assert!(!fresh.stale);
        assert_eq!((fresh.width, fresh.height), (10.0, 20.0));
        let edited = fence("d2", "a -> c");
        assert!(figures.kroki_cached(&edited, Some("doc#0")).unwrap().stale);
        assert!(figures.kroki_cached(&edited, Some("doc#1")).is_err());
    }

    #[test]
    fn content_keys_are_stable_and_distinct() {
        let a = content_key(&fence("d2", "a"));
        assert_eq!(a, content_key(&fence("d2", "a")));
        assert_ne!(a, content_key(&fence("plantuml", "a")));
        assert_eq!(a.len(), 32);
    }
}
