// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Flame graphs of where request time goes, behind `vellum profile`.
//!
//! The profile is built from the span trees in the ring buffer: every
//! span's *self* time (its duration minus its children's) is attributed to
//! its stack of span names. That shows time spent in Vellum's own
//! operations — authorization, cache, database, blob, render, export,
//! search — on every platform the single binary runs on, with no signal
//! handlers and no unsafe code (decision D92). The output is the folded
//! stack format used by Brendan Gregg's tools, and a self-contained SVG.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::store::Trace;

/// Folded stacks: `root;child;leaf` → self time in microseconds.
pub type Folded = BTreeMap<String, u64>;

fn frame_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c == ';' || c.is_whitespace() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// Adds the self time of every span in `trace` to `folded`.
pub fn fold_into(folded: &mut Folded, trace: &Trace) {
    let by_index: BTreeMap<u32, &crate::store::RecordedSpan> =
        trace.spans.iter().map(|s| (s.index, s)).collect();
    let mut child_time: BTreeMap<u32, u64> = BTreeMap::new();
    for span in &trace.spans {
        if let Some(parent) = span.parent {
            *child_time.entry(parent).or_default() += span.duration_us;
        }
    }
    for span in &trace.spans {
        let own = span
            .duration_us
            .saturating_sub(child_time.get(&span.index).copied().unwrap_or(0));
        if own == 0 {
            continue;
        }
        let mut stack = vec![frame_name(&span.name)];
        let mut cursor = span.parent;
        // A malformed tree cannot loop forever: depth is bounded by size.
        let mut guard = trace.spans.len();
        while let (Some(index), true) = (cursor, guard > 0) {
            guard -= 1;
            let Some(parent) = by_index.get(&index) else {
                break;
            };
            stack.push(frame_name(&parent.name));
            cursor = parent.parent;
        }
        stack.reverse();
        *folded.entry(stack.join(";")).or_default() += own;
    }
}

/// Folds many traces.
#[must_use]
pub fn fold<'a>(traces: impl IntoIterator<Item = &'a Trace>) -> Folded {
    let mut folded = Folded::new();
    for trace in traces {
        fold_into(&mut folded, trace);
    }
    folded
}

/// Formats folded stacks as text, one `stack value` line each.
#[must_use]
pub fn to_text(folded: &Folded) -> String {
    let mut out = String::new();
    for (stack, us) in folded {
        let _ = writeln!(out, "{stack} {us}");
    }
    out
}

/// Parses the folded text format; malformed lines are skipped.
#[must_use]
pub fn from_text(text: &str) -> Folded {
    let mut folded = Folded::new();
    for line in text.lines() {
        if let Some((stack, value)) = line.trim().rsplit_once(' ')
            && let Ok(v) = value.parse::<u64>()
        {
            *folded.entry(stack.to_owned()).or_default() += v;
        }
    }
    folded
}

#[derive(Debug, Default)]
struct Node {
    total: u64,
    children: BTreeMap<String, Self>,
}

impl Node {
    fn depth(&self) -> usize {
        1 + self.children.values().map(Self::depth).max().unwrap_or(0)
    }
}

fn tree(folded: &Folded) -> Node {
    let mut root = Node::default();
    for (stack, us) in folded {
        root.total += us;
        let mut node = &mut root;
        for frame in stack.split(';') {
            node = node.children.entry(frame.to_owned()).or_default();
            node.total += us;
        }
    }
    root
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// A warm colour that is stable for a frame name.
fn colour(name: &str) -> String {
    let hash = name.bytes().fold(2_166_136_261_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16_777_619)
    });
    let r = 205 + (hash % 50);
    let g = 80 + ((hash >> 8) % 120);
    let b = 40 + ((hash >> 16) % 40);
    format!("rgb({r},{g},{b})")
}

const WIDTH: f64 = 1200.0;
const FRAME: f64 = 18.0;
const PAD: f64 = 10.0;
const HEADER: f64 = 36.0;

struct Svg {
    out: String,
    scale: f64,
    height: f64,
    total: u64,
}

impl Svg {
    #[expect(clippy::cast_precision_loss, reason = "microsecond totals fit in f64")]
    fn frame(&mut self, name: &str, node: &Node, x: f64, depth: usize) {
        let w = node.total as f64 * self.scale;
        if w < 0.5 {
            return;
        }
        let y = FRAME.mul_add(-(depth as f64 + 1.0), self.height - PAD);
        let pct = node.total as f64 * 100.0 / self.total.max(1) as f64;
        let ms = node.total as f64 / 1000.0;
        let label = escape(name);
        let _ = write!(
            self.out,
            r#"<g><title>{label} ({ms:.1} ms, {pct:.2}%)</title><rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="2" fill="{fill}"/>"#,
            h = FRAME - 1.0,
            fill = colour(name),
        );
        // About 7 px per character at 12 px; only label frames with room.
        let room = ((w - 6.0) / 7.0).floor();
        if room >= 3.0 {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "positive and small"
            )]
            let chars = room as usize;
            let shown: String = if name.chars().count() > chars {
                let mut s: String = name.chars().take(chars.saturating_sub(1)).collect();
                s.push('…');
                s
            } else {
                name.to_owned()
            };
            let _ = write!(
                self.out,
                r#"<text x="{tx:.1}" y="{ty:.1}">{text}</text>"#,
                tx = x + 3.0,
                ty = y + FRAME - 5.0,
                text = escape(&shown),
            );
        }
        self.out.push_str("</g>");
        let mut cx = x;
        for (child, node) in &node.children {
            self.frame(child, node, cx, depth + 1);
            cx = (node.total as f64).mul_add(self.scale, cx);
        }
    }
}

/// Renders folded stacks as a self-contained SVG flame graph, root at the
/// bottom, widths proportional to time.
#[must_use]
#[expect(clippy::cast_precision_loss, reason = "microsecond totals fit in f64")]
pub fn svg(folded: &Folded, title: &str) -> String {
    let root = tree(folded);
    let depth = root.depth();
    let height = FRAME.mul_add(depth as f64, PAD.mul_add(2.0, HEADER));
    let mut svg = Svg {
        out: String::new(),
        scale: PAD.mul_add(-2.0, WIDTH) / root.total.max(1) as f64,
        height,
        total: root.total,
    };
    let _ = write!(
        svg.out,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" viewBox="0 0 {WIDTH} {height}" font-family="Roboto, sans-serif" font-size="12"><rect width="100%" height="100%" fill="#F7F8FC"/><text x="{PAD}" y="22" font-size="16" fill="#1D1B20">{title} — {ms:.1} ms total</text>"##,
        title = escape(title),
        ms = root.total as f64 / 1000.0,
    );
    let mut x = PAD;
    for (name, node) in &root.children {
        svg.frame(name, node, x, 0);
        x = (node.total as f64).mul_add(svg.scale, x);
    }
    svg.out.push_str("</svg>\n");
    svg.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::{span, trace};

    #[test]
    fn folds_self_time() {
        let mut t = trace("/a", 200, 1000, 0);
        t.spans = vec![
            span(0, None, "http.request", 0, 1000),
            span(1, Some(0), "db query", 100, 600),
            span(2, Some(1), "db;decode", 200, 100),
            span(3, Some(0), "render", 700, 0),
        ];
        let folded = fold([&t]);
        assert_eq!(folded.get("http.request"), Some(&400));
        assert_eq!(folded.get("http.request;db_query"), Some(&500));
        assert_eq!(folded.get("http.request;db_query;db_decode"), Some(&100));
        assert_eq!(folded.len(), 3);
        let text = to_text(&folded);
        assert_eq!(from_text(&format!("{text}garbage\nx notanumber\n")), folded);
    }

    #[test]
    fn missing_parents_stop_the_stack() {
        let mut t = trace("/a", 200, 10, 0);
        t.spans = vec![span(5, Some(99), "orphan", 0, 10)];
        assert_eq!(fold([&t]).get("orphan"), Some(&10));
    }

    #[test]
    fn renders_svg() {
        let mut folded = Folded::new();
        let _ = folded.insert("http.request".into(), 300);
        let _ = folded.insert("http.request;db.query".into(), 650);
        let _ = folded.insert("http.request;tiny".into(), 0);
        let _ = folded.insert("http.request;<script>".into(), 50);
        let out = svg(&folded, "Vellum & co");
        assert!(out.starts_with("<svg"));
        assert!(out.contains("Vellum &amp; co — 1.0 ms total"));
        assert!(out.contains("db.query (0.7 ms, 65.00%)"));
        assert!(out.contains("&lt;script&gt;"));
        assert!(!out.contains("<script>"));
        assert!(out.ends_with("</svg>\n"));
        let empty = svg(&Folded::new(), "empty");
        assert!(empty.contains("0.0 ms total"));
    }

    #[test]
    fn long_names_are_cut() {
        let mut folded = Folded::new();
        let _ = folded.insert("x".repeat(400), 1);
        let _ = folded.insert("narrow".into(), 1);
        assert!(svg(&folded, "t").contains('…'));
        assert_eq!(escape("a\u{7}'"), "a&#39;");
        assert_ne!(colour("a"), colour("b"));
    }
}
