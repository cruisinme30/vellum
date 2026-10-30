// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Allow-list sanitiser for SVG and MathML islands.
//!
//! `ammonia` sanitises HTML but not foreign content, so SVG (charts,
//! Mermaid, Kroki) and MathML (display math) are parsed as XML here and
//! rebuilt from scratch: only listed elements and attributes survive, text
//! is re-escaped, links must point inside the island, CSS is filtered and
//! scoped to the island, and every id is prefixed so two islands on a page
//! never collide. DTDs are refused by the parser, so entity tricks cannot
//! reach this code.

use std::collections::HashSet;

use roxmltree::{Document, Node, NodeType, ParsingOptions};

use crate::html;

/// Why an island was rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IslandError {
    /// The markup is not well-formed XML.
    #[error("not well-formed: {0}")]
    Xml(String),
    /// The root element is not `<svg>` or `<math>`.
    #[error("unexpected root element <{0}>")]
    Root(String),
}

const SVG_ELEMENTS: &[&str] = &[
    "svg",
    "g",
    "defs",
    "symbol",
    "use",
    "path",
    "rect",
    "circle",
    "ellipse",
    "line",
    "polyline",
    "polygon",
    "text",
    "tspan",
    "textPath",
    "title",
    "desc",
    "marker",
    "clipPath",
    "mask",
    "linearGradient",
    "radialGradient",
    "stop",
    "pattern",
    "style",
    "switch",
    "foreignObject",
    "filter",
    "feGaussianBlur",
    "feOffset",
    "feFlood",
    "feComposite",
    "feMerge",
    "feMergeNode",
    "feBlend",
    "feColorMatrix",
    "feDropShadow",
    "feMorphology",
    // XHTML that Mermaid puts in foreignObject labels.
    "div",
    "span",
    "p",
    "br",
    "b",
    "i",
    "strong",
    "em",
    "code",
    "sub",
    "sup",
];

const MATH_ELEMENTS: &[&str] = &[
    "math",
    "semantics",
    "mrow",
    "mi",
    "mn",
    "mo",
    "ms",
    "mtext",
    "mspace",
    "msub",
    "msup",
    "msubsup",
    "munder",
    "mover",
    "munderover",
    "mfrac",
    "msqrt",
    "mroot",
    "mtable",
    "mtr",
    "mtd",
    "mlabeledtr",
    "mstyle",
    "merror",
    "mpadded",
    "mphantom",
    "menclose",
    "mmultiscripts",
    "mprescripts",
    "none",
];

/// Elements whose subtree is dropped entirely, text included.
const DROP_SUBTREE: &[&str] = &[
    "script",
    "iframe",
    "object",
    "embed",
    "annotation",
    "annotation-xml",
];

const SVG_ATTRIBUTES: &[&str] = &[
    "id",
    "class",
    "style",
    "role",
    "aria-label",
    "aria-labelledby",
    "aria-describedby",
    "aria-hidden",
    "focusable",
    "lang",
    "dir",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "fx",
    "fy",
    "width",
    "height",
    "d",
    "points",
    "transform",
    "viewBox",
    "preserveAspectRatio",
    "fill",
    "fill-opacity",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-miterlimit",
    "opacity",
    "color",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "text-anchor",
    "dominant-baseline",
    "alignment-baseline",
    "baseline-shift",
    "letter-spacing",
    "word-spacing",
    "text-decoration",
    "dx",
    "dy",
    "rotate",
    "textLength",
    "lengthAdjust",
    "startOffset",
    "visibility",
    "display",
    "clip-path",
    "clip-rule",
    "mask",
    "marker-start",
    "marker-mid",
    "marker-end",
    "markerWidth",
    "markerHeight",
    "markerUnits",
    "refX",
    "refY",
    "orient",
    "offset",
    "stop-color",
    "stop-opacity",
    "gradientUnits",
    "gradientTransform",
    "spreadMethod",
    "patternUnits",
    "patternContentUnits",
    "patternTransform",
    "clipPathUnits",
    "maskUnits",
    "maskContentUnits",
    "filter",
    "filterUnits",
    "primitiveUnits",
    "in",
    "in2",
    "result",
    "stdDeviation",
    "operator",
    "k1",
    "k2",
    "k3",
    "k4",
    "mode",
    "type",
    "values",
    "flood-color",
    "flood-opacity",
    "radius",
    "href",
    "xmlns",
    "version",
    "data-id",
    "data-node",
    "data-et",
    "data-edge",
    "data-look",
    "data-vl-range",
];

const MATH_ATTRIBUTES: &[&str] = &[
    "display",
    "mathvariant",
    "mathsize",
    "mathcolor",
    "mathbackground",
    "displaystyle",
    "scriptlevel",
    "fence",
    "separator",
    "stretchy",
    "symmetric",
    "largeop",
    "movablelimits",
    "accent",
    "accentunder",
    "lspace",
    "rspace",
    "minsize",
    "maxsize",
    "form",
    "linethickness",
    "numalign",
    "denomalign",
    "bevelled",
    "columnalign",
    "rowalign",
    "columnspan",
    "rowspan",
    "columnlines",
    "rowlines",
    "frame",
    "notation",
    "width",
    "height",
    "depth",
    "voffset",
    "class",
    "id",
    "style",
    "xmlns",
    "role",
    "aria-label",
];

/// Presentation attributes that may carry a `var()` colour; those move into
/// `style`, because CSS variables only work in CSS.
const COLOUR_ATTRIBUTES: &[&str] =
    &["fill", "stroke", "stop-color", "flood-color", "color"];

/// Sanitises an SVG or MathML island and scopes its ids and CSS under
/// `prefix` (which must be a valid id fragment, such as `f3`).
///
/// # Errors
///
/// Returns [`IslandError`] when the input is not well-formed XML or its root
/// is neither `<svg>` nor `<math>`.
pub fn sanitize_island(src: &str, prefix: &str) -> Result<String, IslandError> {
    let options = ParsingOptions {
        allow_dtd: false,
        ..ParsingOptions::default()
    };
    let doc = Document::parse_with_options(src, options)
        .map_err(|e| IslandError::Xml(e.to_string()))?;
    let root = doc.root_element();
    let root_name = root.tag_name().name();
    let (elements, attributes) = match root_name {
        "svg" => (SVG_ELEMENTS, SVG_ATTRIBUTES),
        "math" => (MATH_ELEMENTS, MATH_ATTRIBUTES),
        other => return Err(IslandError::Root(other.to_owned())),
    };
    let ids: HashSet<&str> = root
        .descendants()
        .skip(1)
        .filter_map(|n| n.attribute("id"))
        .collect();
    let mut w = Writer {
        out: String::with_capacity(src.len()),
        prefix,
        ids,
        elements,
        attributes,
        root_id: format!("{prefix}-root"),
    };
    w.element(root, true);
    Ok(w.out)
}

struct Writer<'a> {
    out: String,
    prefix: &'a str,
    ids: HashSet<&'a str>,
    elements: &'static [&'static str],
    attributes: &'static [&'static str],
    root_id: String,
}

impl Writer<'_> {
    fn element(&mut self, node: Node<'_, '_>, is_root: bool) {
        let name = node.tag_name().name();
        if DROP_SUBTREE.contains(&name) {
            return;
        }
        if !self.elements.contains(&name) {
            // Unknown wrappers (such as `<a>`) are unwrapped: their
            // children may still be allowed.
            self.children(node);
            return;
        }
        if name == "style" {
            let css: String = node
                .children()
                .filter(roxmltree::Node::is_text)
                .filter_map(|c| c.text())
                .collect();
            let scoped = self.scope_css(&css);
            if !scoped.is_empty() {
                self.out.push_str("<style>");
                // `<` never survives the filter, so `</style>` cannot occur.
                self.out.push_str(&scoped);
                self.out.push_str("</style>");
            }
            return;
        }
        self.out.push('<');
        self.out.push_str(name);
        let mut style = String::new();
        for a in node.attributes() {
            let attr_name = a.name();
            let value = a.value();
            if attr_name.starts_with("on")
                || !self.attributes.contains(&attr_name)
            {
                continue;
            }
            match attr_name {
                "xmlns" => {}
                "id" => {
                    if is_root {
                        continue;
                    }
                    let id = format!("{}-{}", self.prefix, value);
                    html::push_attr(&mut self.out, "id", &id);
                }
                "href" => {
                    // Only in-island fragment references survive.
                    if let Some(target) =
                        value.strip_prefix('#').filter(|t| self.ids.contains(t))
                    {
                        let v = format!("#{}-{target}", self.prefix);
                        html::push_attr(&mut self.out, "href", &v);
                    }
                }
                "aria-labelledby" | "aria-describedby" => {
                    let refs: Vec<String> = value
                        .split_whitespace()
                        .filter(|t| self.ids.contains(t))
                        .map(|t| format!("{}-{t}", self.prefix))
                        .collect();
                    if !refs.is_empty() {
                        html::push_attr(
                            &mut self.out,
                            attr_name,
                            &refs.join(" "),
                        );
                    }
                }
                "style" => {
                    let filtered = filter_declarations(value);
                    let rewritten = self.rewrite_refs(&filtered);
                    style.push_str(&rewritten);
                }
                _ if COLOUR_ATTRIBUTES.contains(&attr_name)
                    && value.contains("var(") =>
                {
                    if is_safe_value(value) {
                        style.push_str(attr_name);
                        style.push(':');
                        style.push_str(value);
                        style.push(';');
                    }
                }
                _ => {
                    if is_safe_value(value) {
                        let rewritten = self.rewrite_refs(value);
                        html::push_attr(&mut self.out, attr_name, &rewritten);
                    }
                }
            }
        }
        if is_root {
            let root_id = self.root_id.clone();
            html::push_attr(&mut self.out, "id", &root_id);
            if name == "svg" {
                self.out.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
            } else {
                self.out
                    .push_str(" xmlns=\"http://www.w3.org/1998/Math/MathML\"");
            }
        }
        if !style.is_empty() {
            html::push_attr(&mut self.out, "style", &style);
        }
        if node.has_children() {
            self.out.push('>');
            self.children(node);
            self.out.push_str("</");
            self.out.push_str(name);
            self.out.push('>');
        } else if name == "br" {
            self.out.push_str("/>");
        } else {
            self.out.push_str("></");
            self.out.push_str(name);
            self.out.push('>');
        }
    }

    fn children(&mut self, node: Node<'_, '_>) {
        for child in node.children() {
            match child.node_type() {
                NodeType::Element => self.element(child, false),
                NodeType::Text => {
                    html::text(&mut self.out, child.text().unwrap_or_default())
                }
                NodeType::Root | NodeType::Comment | NodeType::PI => {}
            }
        }
    }

    /// Rewrites `url(#id)` references to the prefixed ids.
    fn rewrite_refs(&self, value: &str) -> String {
        if !value.contains("url(") {
            return value.to_owned();
        }
        let mut out = String::with_capacity(value.len() + 8);
        let mut rest = value;
        while let Some(i) = rest.find("url(#") {
            let (head, tail) = rest.split_at(i);
            out.push_str(head);
            let after = tail.get(5..).unwrap_or_default();
            let end = after.find(')').unwrap_or(after.len());
            let id = after
                .get(..end)
                .unwrap_or_default()
                .trim_matches(['\'', '"']);
            if self.ids.contains(id) {
                out.push_str("url(#");
                out.push_str(self.prefix);
                out.push('-');
                out.push_str(id);
                out.push(')');
            } else {
                out.push_str("none");
            }
            rest = after.get(end + 1..).unwrap_or_default();
        }
        out.push_str(rest);
        out
    }

    /// Filters a stylesheet and scopes every rule under the island's root.
    fn scope_css(&self, css: &str) -> String {
        let css = strip_css_comments(css);
        let mut out = String::new();
        let mut rest = css.as_str();
        while let Some(open) = rest.find('{') {
            let head = rest.get(..open).unwrap_or_default();
            // Statement at-rules such as `@import url(...);` end in `;`
            // before the next block; drop them.
            if let Some(semi) = head.rfind(';') {
                rest = rest.get(semi + 1..).unwrap_or_default();
                continue;
            }
            let selectors = head.trim();
            let body_start = open + 1;
            // At-rules (@import, @font-face, @keyframes, @media...) are
            // dropped with their whole block.
            if selectors.starts_with('@') {
                let end = matching_brace(rest, open).unwrap_or(rest.len());
                rest = rest.get(end + 1..).unwrap_or_default();
                continue;
            }
            let Some(close) = rest.get(body_start..).and_then(|r| r.find('}'))
            else {
                break;
            };
            let body =
                rest.get(body_start..body_start + close).unwrap_or_default();
            rest = rest.get(body_start + close + 1..).unwrap_or_default();
            let decls = self.rewrite_refs(&filter_declarations(body));
            if decls.is_empty()
                || selectors.is_empty()
                || !is_safe_selector(selectors)
            {
                continue;
            }
            let scoped: Vec<String> = selectors
                .split(',')
                .map(|s| self.scope_selector(s.trim()))
                .collect();
            out.push_str(&scoped.join(","));
            out.push('{');
            out.push_str(&decls);
            out.push('}');
        }
        out
    }

    fn scope_selector(&self, selector: &str) -> String {
        // Rewrite `#id` tokens to prefixed ids; the original root id maps to
        // the island root.
        let mut rewritten = String::with_capacity(selector.len() + 16);
        let mut chars = selector.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '#' {
                let start = i + 1;
                let mut end = start;
                while let Some(&(j, d)) = chars.peek() {
                    if d.is_ascii_alphanumeric() || d == '-' || d == '_' {
                        end = j + d.len_utf8();
                        let _ = chars.next();
                    } else {
                        break;
                    }
                }
                let id = selector.get(start..end).unwrap_or_default();
                rewritten.push('#');
                if self.is_root_id(id) {
                    rewritten.push_str(&self.root_id);
                } else {
                    rewritten.push_str(self.prefix);
                    rewritten.push('-');
                    rewritten.push_str(id);
                }
            } else {
                rewritten.push(c);
            }
        }
        let root = format!("#{}", self.root_id);
        if rewritten.starts_with(&root) {
            rewritten
        } else {
            format!("{root} {rewritten}")
        }
    }

    fn is_root_id(&self, id: &str) -> bool {
        // The island root gets a fresh id, so the source root's id, and any
        // id that names nothing in the island, point at the root.
        !self.ids.contains(id)
    }
}

fn matching_brace(s: &str, open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices().skip_while(|(i, _)| *i < open) {
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(i) = rest.find("/*") {
        out.push_str(rest.get(..i).unwrap_or_default());
        match rest.get(i + 2..).and_then(|r| r.find("*/")) {
            Some(j) => rest = rest.get(i + 2 + j + 2..).unwrap_or_default(),
            None => rest = "",
        }
    }
    out.push_str(rest);
    out
}

fn is_safe_selector(s: &str) -> bool {
    !s.contains(['<', '\\', '@', '{', '}', '"', '\''])
}

/// Keeps only declarations whose value cannot load anything or escape the
/// island.
#[must_use]
pub fn filter_declarations(decls: &str) -> String {
    let mut out = String::new();
    for decl in decls.split(';') {
        let Some((prop, value)) = decl.split_once(':') else {
            continue;
        };
        let prop = prop.trim().to_ascii_lowercase();
        let value = value.trim();
        let blocked_prop = matches!(
            prop.as_str(),
            "position"
                | "behavior"
                | "-moz-binding"
                | "content"
                | "cursor"
                | "z-index"
        );
        if prop.is_empty()
            || blocked_prop
            || !prop.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            || !is_safe_value(value)
        {
            continue;
        }
        out.push_str(&prop);
        out.push(':');
        out.push_str(value);
        out.push(';');
    }
    out
}

/// A value is safe when it has no markup, escapes, script or external URL.
fn is_safe_value(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    if value.contains(['<', '>', '\\', '{', '}'])
        || lower.contains("expression")
        || lower.contains("javascript:")
        || lower.contains("@import")
        || lower.contains("image-set")
    {
        return false;
    }
    // url() may only point at a fragment inside the island.
    let mut rest = lower.as_str();
    while let Some(i) = rest.find("url(") {
        let after = rest.get(i + 4..).unwrap_or_default().trim_start();
        let after = after.trim_start_matches(['\'', '"']);
        if !after.starts_with('#') {
            return false;
        }
        rest = after;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean(src: &str) -> String {
        sanitize_island(src, "f1").unwrap()
    }

    #[test]
    fn keeps_drawing_elements_and_prefixes_ids() {
        let out = clean("<svg xmlns=\"http://www.w3.org/2000/svg\" id=\"m\" \
            viewBox=\"0 0 10 10\"><defs><marker id=\"arrow\"><path d=\"M0 0L5 \
            5\"/></marker></defs><path d=\"M1 1\" \
            marker-end=\"url(#arrow)\"/><use href=\"#arrow\"/></svg>");
        assert!(
            out.starts_with("<svg viewBox=\"0 0 10 10\" id=\"f1-root\""),
            "{out}"
        );
        assert!(out.contains("<marker id=\"f1-arrow\">"));
        assert!(out.contains("marker-end=\"url(#f1-arrow)\""));
        assert!(out.contains("<use href=\"#f1-arrow\">"));
    }

    #[test]
    fn removes_scripts_handlers_and_external_links() {
        let out = clean("<svg xmlns=\"http://www.w3.org/2000/svg\" \
            xmlns:xlink=\"http://www.w3.org/1999/xlink\"><script>alert(1)</scri\
            pt><a xlink:href=\"javascript:alert(1)\"><rect onload=\"alert(1)\" \
            width=\"5\"/></a><image href=\"https://evil.example/x.png\"/><use \
            href=\"https://evil.example/s.svg#a\"/><foreignObject><iframe \
            src=\"x\"/><div>ok</div></foreignObject></svg>");
        assert!(!out.contains("script"), "{out}");
        assert!(!out.contains("alert"), "{out}");
        assert!(!out.contains("evil"), "{out}");
        assert!(!out.contains("iframe"), "{out}");
        assert!(out.contains("<rect width=\"5\"></rect>"), "{out}");
        assert!(out.contains("<div>ok</div>"), "{out}");
    }

    #[test]
    fn filters_and_scopes_css() {
        let out = clean("<svg xmlns=\"http://www.w3.org/2000/svg\" \
            id=\"mermaid-1\"><style>@import url(https://evil.example/x.css); \
            #mermaid-1 .node rect{fill:#fff;stroke:var(--vl-outline)} \
            body{background:url(https://evil.example/t)} \
            .label{color:red;position:fixed} @media \
            print{svg{display:none}}</style><rect class=\"node\"/></svg>");
        assert!(!out.contains("evil"), "{out}");
        assert!(!out.contains('@'), "{out}");
        assert!(!out.contains("position"), "{out}");
        assert!(
            out.contains(
                "#f1-root .node rect{fill:#fff;stroke:var(--vl-outline);}"
            ),
            "{out}"
        );
        assert!(out.contains("#f1-root .label{color:red;}"), "{out}");
    }

    #[test]
    fn moves_var_colours_into_style() {
        let out = clean("<svg xmlns=\"http://www.w3.org/2000/svg\"><rect \
            fill=\"var(--vl-chart-1)\" stroke=\"#000\"/></svg>");
        assert!(
            out.contains(
                r##"<rect stroke="#000" style="fill:var(--vl-chart-1);">"##
            ),
            "{out}"
        );
    }

    #[test]
    fn rejects_dtds_and_foreign_roots() {
        let xxe = "<?xml version=\"1.0\"?><!DOCTYPE svg [<!ENTITY x SYSTEM \
            \"file:///etc/passwd\">]><svg>&x;</svg>";
        assert!(matches!(
            sanitize_island(xxe, "f"),
            Err(IslandError::Xml(_))
        ));
        assert!(matches!(
            sanitize_island("<html/>", "f"),
            Err(IslandError::Root(_))
        ));
        assert!(matches!(
            sanitize_island("<svg>", "f"),
            Err(IslandError::Xml(_))
        ));
    }

    #[test]
    fn keeps_mathml() {
        let out = clean("<math xmlns=\"http://www.w3.org/1998/Math/MathML\" \
            display=\"block\"><msub><mi>F</mi><mn>1</mn></msub><annotation \
            encoding=\"application/x-tex\">F_1</annotation></math>");
        assert_eq!(
            out,
            "<math display=\"block\" id=\"f1-root\" \
                xmlns=\"http://www.w3.org/1998/Math/MathML\"><msub><mi>F</mi><m\
                n>1</mn></msub></math>"
        );
    }
}
