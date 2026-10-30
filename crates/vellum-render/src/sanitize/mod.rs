// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The sanitising pass every rendered page goes through.
//!
//! The renderer already escapes all user text, so this is defence in depth:
//! the page's HTML is re-parsed by `ammonia` with a strict allow-list, and
//! SVG and MathML islands (which `ammonia` would strip) are sanitised
//! separately by [`xml`] and spliced back into placeholders afterwards.

pub mod xml;

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use ammonia::Builder;

/// Tags allowed in rendered pages.
const TAGS: &[&str] = &[
    "a",
    "abbr",
    "article",
    "aside",
    "b",
    "blockquote",
    "br",
    "caption",
    "code",
    "dd",
    "del",
    "details",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "iframe",
    "img",
    "input",
    "label",
    "li",
    "mark",
    "meter",
    "nav",
    "ol",
    "output",
    "p",
    "pre",
    "progress",
    "s",
    "section",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "ul",
];

/// Attributes allowed on every tag.
const GENERIC: &[&str] = &[
    "class",
    "id",
    "title",
    "lang",
    "dir",
    "role",
    "hidden",
    "aria-label",
    "aria-hidden",
    "aria-labelledby",
    "aria-describedby",
    "aria-level",
    "aria-valuenow",
    "aria-valuemin",
    "aria-valuemax",
    "aria-sort",
    "aria-current",
    "tabindex",
];

/// `data-*` attributes the client hydrates; anything else is dropped.
const DATA: &[&str] = &[
    "data-vl-island",
    "data-block",
    "data-handle",
    "data-issue",
    "data-target",
    "data-date",
    "data-relative",
    "data-lang",
    "data-line",
    "data-sortable",
    "data-bar",
    "data-sum",
    "data-formula",
    "data-unit",
    "data-key",
    "data-collection",
    "data-view",
    "data-filter",
    "data-source",
    "data-kind",
    "data-name",
    "data-trend",
    "data-provider",
    "data-stale",
    "data-cols",
    "data-index",
    "data-x",
    "data-y",
];

static POLICY: LazyLock<Builder<'static>> = LazyLock::new(|| {
    let mut b = Builder::empty();
    let _ = b
        .tags(TAGS.iter().copied().collect())
        .generic_attributes(GENERIC.iter().chain(DATA).copied().collect())
        .tag_attributes(tag_attributes())
        .tag_attribute_values(tag_attribute_values())
        .set_tag_attribute_value(
            "iframe",
            "sandbox",
            "allow-scripts allow-same-origin allow-popups",
        )
        .url_schemes(["http", "https", "mailto", "tel"].into_iter().collect())
        .url_relative(ammonia::UrlRelative::PassThrough)
        .link_rel(Some("noopener noreferrer nofollow ugc"))
        .strip_comments(true)
        .attribute_filter(|element, attribute, value| {
            // Embedded frames may only load https URLs from allow-listed
            // providers; the renderer only emits those, so anything else
            // was injected.
            if element == "iframe" && attribute == "src" {
                return crate::embed::is_allowed_frame_src(value)
                    .then(|| value.into());
            }
            if element == "img"
                && attribute == "src"
                && value.trim_start().to_ascii_lowercase().starts_with("data:")
            {
                return None;
            }
            Some(value.into())
        });
    b
});

fn tag_attributes() -> HashMap<&'static str, HashSet<&'static str>> {
    let set = |a: &[&'static str]| a.iter().copied().collect::<HashSet<_>>();
    [
        ("a", set(&["href"])),
        (
            "img",
            set(&["src", "alt", "width", "height", "loading", "decoding"]),
        ),
        ("iframe", set(&["src", "allow", "allowfullscreen"])),
        (
            "input",
            set(&["checked", "disabled", "min", "max", "step", "value"]),
        ),
        ("label", set(&["for"])),
        (
            "meter",
            set(&["min", "max", "value", "low", "high", "optimum"]),
        ),
        ("progress", set(&["max", "value"])),
        ("output", set(&["for"])),
        ("ol", set(&["start", "reversed"])),
        ("td", set(&["colspan", "rowspan", "headers"])),
        (
            "th",
            set(&["colspan", "rowspan", "headers", "scope", "abbr"]),
        ),
        ("time", set(&["datetime"])),
        ("details", set(&["open"])),
        ("abbr", set(&["title"])),
    ]
    .into_iter()
    .collect()
}

fn tag_attribute_values()
-> HashMap<&'static str, HashMap<&'static str, HashSet<&'static str>>> {
    let mut input = HashMap::new();
    let _ = input.insert("type", ["checkbox", "range"].into_iter().collect());
    // The writer sets these; only these values survive. `sandbox` is forced
    // (one forced attribute keeps the output order stable).
    let mut iframe = HashMap::new();
    let _ = iframe
        .insert("referrerpolicy", std::iter::once("no-referrer").collect());
    let _ = iframe.insert("loading", std::iter::once("lazy").collect());
    [("input", input), ("iframe", iframe)].into_iter().collect()
}

/// Sanitises a rendered page and splices its islands back in.
///
/// `islands[i]` replaces the placeholder `<span data-vl-island="i"></span>`;
/// each must already have been through [`xml::sanitize_island`].
#[must_use]
pub fn page(html: &str, islands: &[String]) -> String {
    const OPEN: &str = "<span data-vl-island=\"";
    let clean = POLICY.clean(html).to_string();
    if islands.is_empty() {
        return clean;
    }
    let mut out = String::with_capacity(
        clean.len() + islands.iter().map(String::len).sum::<usize>(),
    );
    let mut rest = clean.as_str();
    while let Some(i) = rest.find(OPEN) {
        out.push_str(rest.get(..i).unwrap_or_default());
        let after = rest.get(i + OPEN.len()..).unwrap_or_default();
        let parsed = after.find("\"></span>").and_then(|end| {
            let n: usize = after.get(..end)?.parse().ok()?;
            Some((n, end))
        });
        match parsed.and_then(|(n, end)| Some((islands.get(n)?, end))) {
            Some((island, end)) => {
                out.push_str(island);
                rest =
                    after.get(end + "\"></span>".len()..).unwrap_or_default();
            }
            None => {
                out.push_str(OPEN);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_disallowed_markup() {
        let out = page(
            concat!(
                r#"<p onclick="x()">a<script>alert(1)</script><a "#,
                r#"href="javascript:alert(1)">l</a><img "#,
                r#"src="data:image/png;base64,AA"><input "#,
                r#"type="text"><style>p{}</style></p>"#,
            ),
            &[],
        );
        assert!(!out.contains("onclick"));
        assert!(!out.contains("script"));
        assert!(!out.contains("javascript"));
        assert!(!out.contains("data:"));
        assert!(!out.contains("type=\"text\""));
        assert!(!out.contains("<style"));
    }

    #[test]
    fn forces_safe_link_and_frame_attributes() {
        let out = page(
            concat!(
                r#"<a href="https://northstar.dev">x</a><iframe "#,
                r#"src="https://www.youtube-nocookie.com/embed/abc" "#,
                r#"sandbox="allow-top-navigation"></iframe><iframe "#,
                r#"src="https://evil.example/"></iframe>"#,
            ),
            &[],
        );
        assert!(
            out.contains(r#"rel="noopener noreferrer nofollow ugc""#),
            "{out}"
        );
        assert!(
            out.contains("allow-scripts allow-same-origin allow-popups"),
            "{out}"
        );
        assert!(!out.contains("allow-top-navigation"), "{out}");
        assert!(!out.contains("evil"), "{out}");
    }

    #[test]
    fn splices_islands_into_placeholders_only() {
        let html = concat!(
            r#"<figure><span data-vl-island="0"></span></figure><p>&lt;span "#,
            r#"data-vl-island="0"&gt;&lt;/span&gt;</p>"#,
        );
        let out = page(html, &["<svg></svg>".to_owned()]);
        assert_eq!(
            out,
            concat!(
                r#"<figure><svg></svg></figure><p>&lt;span "#,
                r#"data-vl-island="0"&gt;&lt;/span&gt;</p>"#,
            )
        );
        // Out-of-range placeholders are left empty, never panic.
        let out = page(
            r#"<span data-vl-island="7"></span>"#,
            &["<svg></svg>".to_owned()],
        );
        assert!(!out.contains("<svg"));
    }
}
