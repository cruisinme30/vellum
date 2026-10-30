// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The golden HTML corpus, sanitiser guarantees over the whole output, and
//! the render budget.
//!
//! Every `.md` under `corpus/render/` renders (offline, with no resolver)
//! to its `.html` sibling. Run with `VELLUM_BLESS=1` to write the expected
//! files after a reviewed change.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helpers"
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use vellum_format::parse;
use vellum_render::figure::{FigureConfig, Figures};
use vellum_render::resolve::{
    IssueCard, IssueGroup, IssueLayout, IssueRef, IssueView, IssuesAt,
    NoResolver, Resolved, Resolver,
};
use vellum_render::{RenderOptions, RenderedPage, Renderer};

fn renderer() -> Renderer {
    Renderer::new(Figures::new(FigureConfig::default()).unwrap())
}

fn render(src: &str) -> RenderedPage {
    renderer().render_offline(
        &parse(src).document,
        &NoResolver,
        &RenderOptions::default(),
    )
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/render")
}

fn inputs() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus())
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|e| e == "md")
                && !p.ends_with("README.md")
        })
        .collect();
    files.sort();
    files
}

/// The tags of sanitised HTML: every raw `<` starts one, because text is
/// escaped.
fn tags(html: &str) -> impl Iterator<Item = &str> {
    html.split('<')
        .skip(1)
        .map(|t| t.split('>').next().unwrap_or_default())
}

/// Things that must never appear in rendered markup, whatever the input.
fn assert_safe(name: &str, html: &str) {
    for tag in tags(html) {
        let lower = tag.to_ascii_lowercase();
        let element = lower.split([' ', '/']).next().unwrap_or_default();
        assert!(
            !matches!(
                element,
                "script"
                    | "object"
                    | "embed"
                    | "base"
                    | "meta"
                    | "link"
                    | "form"
            ),
            "{name}: <{tag}>"
        );
        for bad in [
            "javascript:",
            "vbscript:",
            "data:text",
            "data:image/svg",
            "expression(",
        ] {
            assert!(!lower.contains(bad), "{name}: {bad} in <{tag}>");
        }
        for attr in lower.split_whitespace().skip(1) {
            assert!(
                !attr.starts_with("on"),
                "{name}: event handler in <{tag}>"
            );
        }
        if element == "iframe" {
            assert!(lower.contains("sandbox="), "{name}: unsandboxed <{tag}>");
        }
    }
    // Only SVG islands carry a <style>, scoped to the island and free of
    // imports and external URLs.
    for css in html
        .split("<style")
        .skip(1)
        .map(|s| s.split("</style>").next().unwrap_or_default())
    {
        let lower = css.to_ascii_lowercase();
        assert!(
            !lower.contains("@import") && !lower.contains("javascript"),
            "{name}: {css}"
        );
        assert!(
            lower.split("url(").skip(1).all(|u| u.starts_with('#')),
            "{name}: external url in {css}"
        );
        assert!(lower.contains("-root"), "{name}: unscoped style {css}");
    }
}

#[test]
fn golden_html_corpus() {
    let bless = std::env::var_os("VELLUM_BLESS").is_some();
    let files = inputs();
    assert!(files.len() >= 6, "corpus/render looks empty");
    let mut failures = Vec::new();
    for input in &files {
        let name = input.file_name().unwrap().to_string_lossy().into_owned();
        let page = render(&fs::read_to_string(input).unwrap());
        assert_safe(&name, &page.html);
        let mut out = page.html.replace("><", ">\n<");
        out.push('\n');
        let expected = input.with_extension("html");
        if bless {
            fs::write(&expected, &out).unwrap();
        }
        if fs::read_to_string(&expected).unwrap_or_default() != out {
            failures.push(format!(
                "{name}: HTML differs from {}",
                expected.display()
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{}\nRun with VELLUM_BLESS=1 after reviewing the change.",
        failures.join("\n")
    );
}

#[test]
fn hostile_input_is_neutralised() {
    let src = fs::read_to_string(corpus().join("xss.md")).unwrap();
    let page = render(&src);
    let html = &page.html;
    assert_safe("xss.md", html);
    // Raw HTML stays visible as text.
    assert!(
        html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "{html}"
    );
    // Code fences keep their text.
    assert!(html.contains("code fences are text"));
    // The refused link and image still say what they were.
    assert!(html.contains("javascript link"));
    assert!(
        page.warnings.iter().any(|w| w.message.contains("link to")),
        "{:?}",
        page.warnings
    );
    assert!(
        page.warnings
            .iter()
            .any(|w| w.message.contains("cannot be embedded"))
    );
    // Invisible and bidi control characters are dropped.
    assert!(!html.contains('\u{202e}') && !html.contains('\u{7}'));
}

#[test]
fn every_block_renders_without_falling_back_to_source() {
    let src = fs::read_to_string(corpus().join("fences.md")).unwrap();
    let page = render(&src);
    for class in [
        "vl-chart",
        "vl-timeline",
        "vl-roadmap",
        "vl-risk",
        "vl-board",
        "vl-datatable",
        "vl-architecture",
        "vl-calculator",
        "vl-image",
        "vl-collection",
    ] {
        assert!(page.html.contains(class), "missing {class}");
    }
    // Mermaid is drawn in-process.
    assert!(page.html.contains("data-kind=\"mermaid\""));
    let mermaid = page.html.split("data-kind=\"mermaid\"").nth(1).unwrap();
    assert!(
        mermaid.split("</figure>").next().unwrap().contains("<svg"),
        "mermaid did not draw"
    );
    // Without Kroki, D2 and PlantUML show their source and say why.
    assert!(page.html.contains("gateway -&gt; engine"));
    assert!(
        page.warnings.iter().any(|w| w.message.contains("Kroki")),
        "{:?}",
        page.warnings
    );
    // Calculators are computed on the server too.
    assert!(page.html.contains("1,200"), "calculator result");
}

#[test]
fn islands_get_unique_ids() {
    let page = render(
        "```chart\nvalues: [1, 2]\n```\n\n```chart\nvalues: [3, 4]\n```\n",
    );
    assert!(page.html.contains("id=\"f0-root\""));
    assert!(page.html.contains("id=\"f1-root\""));
    assert!(page.html.contains("aria-labelledby=\"f1-title\""));
}

#[test]
fn outline_and_title() {
    let page = render(
        "---\ntitle: Gateway rollout\n---\n# Overview\n\n::: toc\n:::\n\n\
            ## Plan\n\n### Risks\n\n## Plan\n",
    );
    assert_eq!(page.title.as_deref(), Some("Gateway rollout"));
    let ids: Vec<&str> = page.toc.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, ["u-overview", "u-plan", "u-risks", "u-plan-2"]);
    assert!(page.html.contains("href=\"#u-plan-2\""), "{}", page.html);
    let untitled = render("# First heading\n\ntext\n");
    assert_eq!(untitled.title.as_deref(), Some("First heading"));
}

struct Workspace;

impl Resolver for Workspace {
    fn wikilink(&self, target: &str) -> Option<Resolved> {
        (target == "Gateway rollout").then(|| Resolved {
            href: "/d/01JB".into(),
            title: "Gateway rollout".into(),
        })
    }

    fn mention(&self, handle: &str) -> Option<String> {
        (handle == "maya").then(|| "Maya Chen".into())
    }

    fn issue(&self, key: &str) -> Option<IssueRef> {
        (key == "ENG-142").then(|| IssueRef {
            href: "/i/ENG-142".into(),
            title: "Replay engine".into(),
            state: "In review".into(),
        })
    }
}

#[test]
fn references_resolve_through_the_resolver() {
    let doc = parse(
        "See [[Gateway rollout]] and [[Missing page]], ask @maya about \
            ENG-142.\n",
    )
    .document;
    let page =
        renderer().render_offline(&doc, &Workspace, &RenderOptions::default());
    assert!(page.html.contains("href=\"/d/01JB\""), "{}", page.html);
    assert!(page.html.contains("vl-unresolved"));
    assert!(page.html.contains("@Maya Chen"));
    assert!(page.html.contains("ENG-142 · Replay engine · In review"));
}

#[test]
fn render_is_deterministic_and_blob_is_stable() {
    let src = fs::read_to_string(corpus().join("page.md")).unwrap();
    let a = render(&src).to_blob();
    let b = render(&src).to_blob();
    assert_eq!(a, b);
    assert!(a.etag.starts_with("sha256-"));
    assert_eq!(a.content_type, "text/html; charset=utf-8");
    assert!(vellum_render::blob_key("abc").ends_with("/abc.html"));
}

#[test]
fn deep_nesting_does_not_overflow() {
    let src = "> ".repeat(200) + "deep\n";
    let page = render(&src);
    assert!(page.html.contains("deep"));
    let containers = "::: callout\n".repeat(100) + "x\n" + &":::\n".repeat(100);
    let _ = render(&containers);
}

/// Brief §4: p95 under 300 ms for a 50-block page (release builds; debug
/// builds get ten times the budget).
#[test]
fn fifty_block_page_renders_within_budget() {
    let corpus = fs::read_to_string(corpus().join("fences.md")).unwrap();
    let mut src = String::new();
    let mut blocks = 0;
    while blocks < 50 {
        src.push_str(&corpus);
        src.push('\n');
        blocks = parse(&src).document.blocks.len();
    }
    let doc = parse(&src).document;
    let r = renderer();
    // Warm-up: syntax sets, Mermaid and caches.
    let _ = r.render_offline(&doc, &NoResolver, &RenderOptions::default());
    let mut times: Vec<Duration> = (0..20)
        .map(|_| {
            let t = Instant::now();
            let _ =
                r.render_offline(&doc, &NoResolver, &RenderOptions::default());
            t.elapsed()
        })
        .collect();
    times.sort();
    let p95 = times[18];
    let budget = if cfg!(debug_assertions) {
        Duration::from_secs(3)
    } else {
        Duration::from_millis(300)
    };
    assert!(
        p95 < budget,
        "p95 {p95:?} for {blocks} blocks (budget {budget:?})"
    );
}

#[tokio::test]
async fn async_render_without_kroki_matches_offline() {
    let doc = parse("```d2\na -> b\n```\n\nText.\n").document;
    let r = renderer();
    let opts = RenderOptions {
        doc_key: Some("01JB".into()),
    };
    let online = r.render(&doc, &NoResolver, &opts).await;
    let offline = r.render_offline(&doc, &NoResolver, &opts);
    assert_eq!(online, offline);
}

/// Author-chosen ids must not shadow globals (`window.exports`,
/// `document.location`, ...) through DOM clobbering.
#[test]
fn ids_cannot_clobber_globals() {
    let page = render(
        "# exports\n\n## location {#module}\n\nSee [the plan](#exports).\n\n\
            ```calculator\nname: Name = 1\nout: Out = name * 2\n```\n",
    );
    for tag in tags(&page.html) {
        for attr in ["id=\"", "name=\""] {
            if let Some(rest) = tag.split(attr).nth(1) {
                let value = rest.split('"').next().unwrap();
                assert!(
                    value.contains('-'),
                    "{value} can clobber a global in <{tag}>"
                );
            }
        }
    }
    assert!(page.html.contains("href=\"#u-exports\""), "{}", page.html);
}

struct Issues;

impl Resolver for Issues {
    fn issues(&self, at: IssuesAt<'_>) -> Option<Result<IssueView, String>> {
        let card = |key: &str, href: &str| IssueCard {
            key: key.into(),
            href: href.into(),
            title: format!("{key} <b>title</b>"),
            state: "In review".into(),
            assignee: Some("Maya Chen".into()),
            priority: Some("Urgent".into()),
        };
        match (at.index, at.block_id) {
            (0, None) => Some(Ok(IssueView {
                title: Some("Gateway work".into()),
                layout: IssueLayout::List,
                groups: vec![
                    IssueGroup {
                        title: "In review".into(),
                        issues: vec![card("ENG-142", "/i/ENG-142")],
                    },
                    IssueGroup {
                        title: "Empty".into(),
                        issues: Vec::new(),
                    },
                ],
                more: 3,
            })),
            (1, Some("blk-a1b2c3d4")) => Some(Ok(IssueView {
                title: None,
                layout: IssueLayout::Board,
                groups: vec![
                    IssueGroup {
                        title: "Todo".into(),
                        issues: vec![card("ENG-7", "javascript:alert(1)")],
                    },
                    IssueGroup {
                        title: String::new(),
                        issues: Vec::new(),
                    },
                ],
                more: 0,
            })),
            (2, _) => Some(Err("needs a view or a filter".into())),
            _ => None,
        }
    }
}

#[test]
fn issue_blocks_render_live_views() {
    let src = "::: issues\nfilter:\n  field: status\n:::\n\n:::: tip\n\
        ::: issues ^blk-a1b2c3d4\nlayout: board\n:::\n::::\n\n```md\n\
        ::: issues\n:::\n```\n\n::: issues\nbad: 1\n:::\n\n::: issues\n\
        limit: 5\n:::\n";
    let page = renderer().render_offline(
        &parse(src).document,
        &Issues,
        &RenderOptions::default(),
    );
    let html = &page.html;
    assert_safe("issues", html);
    // The list view, with its heading, groups, cards and the overflow note.
    assert!(html.contains("vl-issues-list"), "{html}");
    assert!(html.contains("Gateway work"));
    assert!(html.contains("href=\"/i/ENG-142\""));
    assert!(
        html.contains("ENG-142 &lt;b&gt;title&lt;/b&gt;"),
        "titles are text"
    );
    assert!(html.contains("3 more issues not shown."));
    assert!(!html.contains(">Empty<"), "empty list groups are hidden");
    // The board nested in a callout is the second block, matched by its id; a
    // hostile link loses its href but keeps its key.
    assert!(html.contains("vl-issues-board"));
    assert!(html.contains("aria-label=\"No value\""));
    assert!(!html.contains("javascript:"));
    assert!(html.contains("data-issue=\"ENG-7\""));
    // The fenced example is not counted; the invalid block warns; the
    // unresolved one waits for the client.
    assert!(
        page.warnings.iter().any(|w| w
            .message
            .contains("issues block 3: needs a view or a filter")),
        "{:?}",
        page.warnings
    );
    assert!(html.contains("data-index=\"3\""));
    assert_eq!(html.matches("vl-live-placeholder").count(), 2, "{html}");
}
