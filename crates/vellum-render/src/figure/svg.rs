// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A tiny SVG writer for the built-in figures.
//!
//! Colours are always `var(--vl-…)` inside `style`, so figures follow the
//! page theme, and [`crate::theme::Theme::resolve_vars`] can bake them in
//! for exports.

use crate::html::{self, num};

/// The font stack used in figures.
pub const FONT: &str = "Roboto, system-ui, sans-serif";

/// An SVG document under construction.
#[derive(Debug)]
pub struct Svg {
    out: String,
    /// Width in user units.
    pub width: f64,
    /// Height in user units.
    pub height: f64,
}

/// A finished drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawing {
    /// The SVG markup.
    pub svg: String,
    /// Width in CSS pixels.
    pub width: f64,
    /// Height in CSS pixels.
    pub height: f64,
    /// A plain-text description for alt text and screen readers.
    pub alt: String,
}

/// Rough text width: Roboto averages about 0.56 em per character.
#[must_use]
pub fn text_width(s: &str, size: f64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let chars = s.chars().count() as f64;
    chars * size * 0.56
}

/// Shortens `s` to fit `width` at `size`, ending in `…`.
#[must_use]
pub fn fit(s: &str, width: f64, size: f64) -> String {
    if text_width(s, size) <= width {
        return s.to_owned();
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let keep = ((width / (size * 0.56)).floor() as usize).saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

impl Svg {
    /// Starts a drawing with an accessible title.
    #[must_use]
    pub fn new(width: f64, height: f64, title: &str) -> Self {
        let mut out = String::with_capacity(4096);
        out.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 ");
        out.push_str(&num(width));
        out.push(' ');
        out.push_str(&num(height));
        out.push_str("\" width=\"");
        out.push_str(&num(width));
        out.push_str("\" height=\"");
        out.push_str(&num(height));
        out.push_str("\" role=\"img\" aria-labelledby=\"title\"");
        out.push_str(" font-family=\"");
        out.push_str(FONT);
        out.push_str("\"><title id=\"title\">");
        html::text(&mut out, title);
        out.push_str("</title>");
        Self { out, width, height }
    }

    /// Appends raw, already-safe markup.
    pub fn raw(&mut self, s: &str) {
        self.out.push_str(s);
    }

    /// A filled rectangle with an optional tooltip.
    #[allow(clippy::too_many_arguments)]
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, rx: f64, fill: &str, tip: Option<&str>) {
        self.out.push_str("<rect x=\"");
        self.out.push_str(&num(x));
        self.out.push_str("\" y=\"");
        self.out.push_str(&num(y));
        self.out.push_str("\" width=\"");
        self.out.push_str(&num(w.max(0.0)));
        self.out.push_str("\" height=\"");
        self.out.push_str(&num(h.max(0.0)));
        if rx > 0.0 {
            self.out.push_str("\" rx=\"");
            self.out.push_str(&num(rx));
        }
        self.out.push_str("\" style=\"fill:");
        self.out.push_str(fill);
        self.close_with_tip("rect", tip);
    }

    /// A rectangle with a stroke and fill.
    #[allow(clippy::too_many_arguments)]
    pub fn boxed(&mut self, x: f64, y: f64, w: f64, h: f64, rx: f64, fill: &str, stroke: &str) {
        self.out.push_str("<rect x=\"");
        self.out.push_str(&num(x));
        self.out.push_str("\" y=\"");
        self.out.push_str(&num(y));
        self.out.push_str("\" width=\"");
        self.out.push_str(&num(w.max(0.0)));
        self.out.push_str("\" height=\"");
        self.out.push_str(&num(h.max(0.0)));
        self.out.push_str("\" rx=\"");
        self.out.push_str(&num(rx));
        self.out.push_str("\" style=\"fill:");
        self.out.push_str(fill);
        self.out.push_str(";stroke:");
        self.out.push_str(stroke);
        self.out.push_str("\"/>");
    }

    /// A straight line.
    #[allow(clippy::too_many_arguments, reason = "two points plus stroke style")]
    pub fn line(
        &mut self,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        stroke: &str,
        width: f64,
        dashed: bool,
    ) {
        self.out.push_str("<line x1=\"");
        self.out.push_str(&num(x1));
        self.out.push_str("\" y1=\"");
        self.out.push_str(&num(y1));
        self.out.push_str("\" x2=\"");
        self.out.push_str(&num(x2));
        self.out.push_str("\" y2=\"");
        self.out.push_str(&num(y2));
        self.out.push_str("\" style=\"stroke:");
        self.out.push_str(stroke);
        self.out.push_str(";stroke-width:");
        self.out.push_str(&num(width));
        if dashed {
            self.out.push_str(";stroke-dasharray:4 4");
        }
        self.out.push_str("\"/>");
    }

    /// A path with fill and stroke.
    pub fn path(&mut self, d: &str, fill: &str, stroke: &str, width: f64, tip: Option<&str>) {
        self.out.push_str("<path d=\"");
        self.out.push_str(d);
        self.out.push_str("\" style=\"fill:");
        self.out.push_str(fill);
        self.out.push_str(";stroke:");
        self.out.push_str(stroke);
        self.out.push_str(";stroke-width:");
        self.out.push_str(&num(width));
        self.out
            .push_str(";stroke-linejoin:round;stroke-linecap:round");
        self.close_with_tip("path", tip);
    }

    /// A circle.
    pub fn circle(&mut self, cx: f64, cy: f64, r: f64, fill: &str, tip: Option<&str>) {
        self.out.push_str("<circle cx=\"");
        self.out.push_str(&num(cx));
        self.out.push_str("\" cy=\"");
        self.out.push_str(&num(cy));
        self.out.push_str("\" r=\"");
        self.out.push_str(&num(r));
        self.out.push_str("\" style=\"fill:");
        self.out.push_str(fill);
        self.close_with_tip("circle", tip);
    }

    /// Text; `anchor` is `start`, `middle` or `end`.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        x: f64,
        y: f64,
        s: &str,
        size: f64,
        fill: &str,
        anchor: &str,
        bold: bool,
    ) {
        self.out.push_str("<text x=\"");
        self.out.push_str(&num(x));
        self.out.push_str("\" y=\"");
        self.out.push_str(&num(y));
        self.out.push_str("\" font-size=\"");
        self.out.push_str(&num(size));
        self.out.push_str("\" text-anchor=\"");
        self.out.push_str(anchor);
        if bold {
            self.out.push_str("\" font-weight=\"600");
        }
        self.out.push_str("\" style=\"fill:");
        self.out.push_str(fill);
        self.out.push_str("\">");
        html::text(&mut self.out, s);
        self.out.push_str("</text>");
    }

    fn close_with_tip(&mut self, tag: &str, tip: Option<&str>) {
        match tip {
            Some(t) => {
                self.out.push_str("\"><title>");
                html::text(&mut self.out, t);
                self.out.push_str("</title></");
                self.out.push_str(tag);
                self.out.push('>');
            }
            None => self.out.push_str("\"/>"),
        }
    }

    /// Finishes the drawing.
    #[must_use]
    pub fn finish(mut self, alt: String) -> Drawing {
        self.out.push_str("</svg>");
        Drawing {
            svg: self.out,
            width: self.width,
            height: self.height,
            alt,
        }
    }
}

/// The CSS colour of categorical series `i`.
#[must_use]
pub fn series_colour(i: usize) -> String {
    format!("var(--vl-chart-{})", i % 8 + 1)
}

/// Text colour.
pub const INK: &str = "var(--vl-on-surface)";
/// Secondary text colour.
pub const MUTED: &str = "var(--vl-on-surface-variant)";
/// Grid and axis colour.
pub const GRID: &str = "var(--vl-outline-variant)";
/// Card and box fill.
pub const CARD: &str = "var(--vl-surface-container-low)";
/// Box outline.
pub const OUTLINE: &str = "var(--vl-outline)";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_escaped_accessible_svg() {
        let mut s = Svg::new(100.0, 50.0, "A <title>");
        s.rect(0.0, 0.0, 10.0, 10.0, 2.0, &series_colour(0), Some("x & y"));
        s.text(5.0, 5.0, "<b>", 12.0, INK, "middle", false);
        let d = s.finish("alt".into());
        assert!(
            d.svg
                .contains("<title id=\"title\">A &lt;title&gt;</title>")
        );
        assert!(d.svg.contains("<title>x &amp; y</title></rect>"));
        assert!(d.svg.contains("&lt;b&gt;</text>"));
        assert!(d.svg.ends_with("</svg>"));
    }

    #[test]
    fn fits_long_labels() {
        assert_eq!(fit("short", 100.0, 12.0), "short");
        let f = fit("a very long label indeed", 40.0, 12.0);
        assert!(f.ends_with('…'));
        assert!(text_width(&f, 12.0) <= 48.0);
    }
}
