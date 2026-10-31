// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `chart` fences: bar, line, area, donut, pie, grouped, stacked, scatter.

use std::f64::consts::{PI, TAU};
use std::fmt::Write as _;

use super::svg::{
    Drawing, GRID, INK, MUTED, Svg, fit, series_colour, text_width,
};
use crate::html::num;
use crate::model::{Chart, ChartKind};

const W: f64 = 640.0;
const PLOT_H: f64 = 220.0;
const LEFT: f64 = 52.0;
const RIGHT: f64 = 16.0;
const TOP: f64 = 16.0;
const AXIS: f64 = 28.0;

/// Formats a value for labels: integers without decimals, otherwise up to
/// two, with thousands separators.
#[must_use]
pub fn format_value(v: f64, unit: &str) -> String {
    let rounded = (v * 100.0).round() / 100.0;
    let mut s = num(rounded);
    // Thousands separators for the integer part.
    let (int, frac) = s.split_once('.').map_or_else(
        || (s.clone(), String::new()),
        |(a, b)| (a.to_owned(), format!(".{b}")),
    );
    let (sign, digits) = int
        .strip_prefix('-')
        .map_or(("", int.as_str()), |d| ("-", d));
    if digits.len() > 3 {
        let mut grouped = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(c);
        }
        s = format!("{sign}{grouped}{frac}");
    }
    if unit.is_empty() {
        s
    } else if unit == "%" {
        format!("{s}%")
    } else {
        format!("{s} {unit}")
    }
}

/// A "nice" axis: rounded bounds and a step of 1, 2 or 5 times a power of
/// ten, about five ticks.
fn nice_axis(min: f64, max: f64) -> (f64, f64, f64) {
    let (min, max) = if (max - min).abs() < f64::EPSILON {
        (min - 1.0, max + 1.0)
    } else {
        (min, max)
    };
    let raw = (max - min) / 5.0;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|m| m * mag)
        .find(|s| *s >= raw)
        .unwrap_or(10.0 * mag);
    (
        (min / step).floor() * step,
        (max / step).ceil() * step,
        step,
    )
}

/// Draws a chart.
#[must_use]
pub fn draw(chart: &Chart) -> Drawing {
    match chart.kind {
        ChartKind::Pie | ChartKind::Donut => radial(chart),
        ChartKind::Scatter => scatter(chart),
        _ => cartesian(chart),
    }
}

fn alt_text(chart: &Chart) -> String {
    let mut alt = String::new();
    let kind = match chart.kind {
        ChartKind::Bar => "Bar chart",
        ChartKind::Line => "Line chart",
        ChartKind::Area => "Area chart",
        ChartKind::Donut => "Donut chart",
        ChartKind::Pie => "Pie chart",
        ChartKind::Grouped => "Grouped bar chart",
        ChartKind::Stacked => "Stacked bar chart",
        ChartKind::Scatter => "Scatter chart",
    };
    alt.push_str(kind);
    if !chart.title.is_empty() {
        let _ = write!(alt, ": {}", chart.title);
    }
    alt.push('.');
    for s in &chart.series {
        if chart.series.len() > 1 {
            let _ = write!(alt, " {}:", s.name);
        }
        let parts: Vec<String> = s
            .values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let label = if chart.kind == ChartKind::Scatter {
                    s.xs.get(i)
                        .map(|x| format_value(*x, ""))
                        .unwrap_or_default()
                } else {
                    chart
                        .labels
                        .get(i)
                        .cloned()
                        .unwrap_or_else(|| (i + 1).to_string())
                };
                format!("{label} {}", format_value(*v, &chart.unit))
            })
            .collect();
        let _ = write!(alt, " {}.", parts.join(", "));
    }
    alt
}

fn legend(svg: &mut Svg, chart: &Chart, y: f64) {
    let mut x = LEFT;
    for (i, s) in chart.series.iter().enumerate() {
        svg.rect(x, y - 9.0, 10.0, 10.0, 2.0, &series_colour(i), None);
        let name = fit(&s.name, 160.0, 12.0);
        svg.text(x + 14.0, y, &name, 12.0, INK, "start", false);
        x += 14.0 + text_width(&name, 12.0) + 18.0;
    }
}

#[allow(clippy::cast_precision_loss)]
fn cartesian(chart: &Chart) -> Drawing {
    let n = chart
        .series
        .iter()
        .map(|s| s.values.len())
        .max()
        .unwrap_or(0)
        .max(chart.labels.len())
        .max(1);
    let stacked = chart.kind == ChartKind::Stacked;
    let totals: Vec<f64> = (0..n)
        .map(|i| {
            chart
                .series
                .iter()
                .filter_map(|s| s.values.get(i))
                .filter(|v| **v > 0.0)
                .sum()
        })
        .collect();
    let all = chart.series.iter().flat_map(|s| s.values.iter().copied());
    let data_max = if stacked {
        totals.iter().copied().fold(0.0, f64::max)
    } else {
        all.clone().fold(f64::MIN, f64::max)
    };
    let data_min = all.fold(f64::MAX, f64::min).min(0.0);
    let (lo, hi, step) = nice_axis(
        chart.ymin.unwrap_or(data_min),
        chart.ymax.unwrap_or_else(|| data_max.max(0.0)),
    );
    let legend_h = if chart.legend { 24.0 } else { 0.0 };
    let height = TOP + PLOT_H + AXIS + legend_h;
    let title = if chart.title.is_empty() {
        "Chart"
    } else {
        chart.title.as_str()
    };
    let mut svg = Svg::new(W, height, title);
    let plot_w = W - LEFT - RIGHT;
    let y_of = |v: f64| TOP + PLOT_H - (v - lo) / (hi - lo) * PLOT_H;

    // Grid and y axis labels.
    let mut tick = lo;
    let mut guard = 0;
    while tick <= hi + step / 2.0 && guard < 50 {
        let y = y_of(tick);
        svg.line(LEFT, y, W - RIGHT, y, GRID, 1.0, false);
        svg.text(
            LEFT - 8.0,
            y + 4.0,
            &format_value(tick, ""),
            11.0,
            MUTED,
            "end",
            false,
        );
        tick += step;
        guard += 1;
    }
    let band = plot_w / n as f64;
    for i in 0..n {
        let label = chart
            .labels
            .get(i)
            .cloned()
            .unwrap_or_else(|| (i + 1).to_string());
        let x = LEFT + band * (i as f64 + 0.5);
        svg.text(
            x,
            TOP + PLOT_H + 18.0,
            &fit(&label, band - 4.0, 11.0),
            11.0,
            MUTED,
            "middle",
            false,
        );
    }
    let zero = y_of(0.0_f64.clamp(lo, hi));

    match chart.kind {
        ChartKind::Line | ChartKind::Area => {
            for (si, s) in chart.series.iter().enumerate() {
                let colour = series_colour(si);
                let pts: Vec<(f64, f64)> = s
                    .values
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (LEFT + band * (i as f64 + 0.5), y_of(*v)))
                    .collect();
                let mut d = String::new();
                for (i, (x, y)) in pts.iter().enumerate() {
                    let _ = write!(
                        d,
                        "{}{} {}",
                        if i == 0 { "M" } else { "L" },
                        num(*x),
                        num(*y)
                    );
                }
                if chart.kind == ChartKind::Area
                    && let (Some(first), Some(last)) = (pts.first(), pts.last())
                {
                    let area = format!(
                        "{d}L{} {}L{} {}Z",
                        num(last.0),
                        num(zero),
                        num(first.0),
                        num(zero)
                    );
                    svg.raw(&format!(
                        "<path d=\"{area}\" \
                            style=\"fill:{colour};fill-opacity:0.2;stroke:none\
                            \"/>"
                    ));
                }
                svg.path(&d, "none", &colour, 2.5, None);
                for (i, (x, y)) in pts.iter().enumerate() {
                    let label =
                        chart.labels.get(i).cloned().unwrap_or_default();
                    let tip = format!(
                        "{label}: {}",
                        format_value(
                            s.values.get(i).copied().unwrap_or(0.0),
                            &chart.unit
                        )
                    );
                    svg.circle(*x, *y, 3.5, &colour, Some(&tip));
                }
            }
        }
        _ => {
            let k = chart.series.len().max(1) as f64;
            let inner = band * 0.7;
            let mut stack_top = vec![0.0_f64; n];
            for (si, s) in chart.series.iter().enumerate() {
                let colour = series_colour(si);
                for (i, v) in s.values.iter().enumerate() {
                    let label =
                        chart.labels.get(i).cloned().unwrap_or_default();
                    let tip = if chart.series.len() > 1 {
                        format!(
                            "{label} · {}: {}",
                            s.name,
                            format_value(*v, &chart.unit)
                        )
                    } else {
                        format!("{label}: {}", format_value(*v, &chart.unit))
                    };
                    let x0 = LEFT + band * i as f64 + (band - inner) / 2.0;
                    let (x, w, top, bottom) = if stacked {
                        let base = stack_top.get(i).copied().unwrap_or(0.0);
                        let next = base + v.max(0.0);
                        if let Some(t) = stack_top.get_mut(i) {
                            *t = next;
                        }
                        (x0, inner, y_of(next), y_of(base))
                    } else if chart.kind == ChartKind::Grouped
                        || chart.series.len() > 1
                    {
                        let w = inner / k;
                        (
                            x0 + w * si as f64,
                            w - 2.0,
                            y_of(v.max(0.0)),
                            y_of(v.min(0.0)),
                        )
                    } else {
                        (x0, inner, y_of(v.max(0.0)), y_of(v.min(0.0)))
                    };
                    svg.rect(x, top, w, bottom - top, 3.0, &colour, Some(&tip));
                }
            }
        }
    }
    svg.line(LEFT, zero, W - RIGHT, zero, MUTED, 1.0, false);
    if chart.legend {
        legend(&mut svg, chart, height - 8.0);
    }
    svg.finish(alt_text(chart))
}

#[allow(clippy::cast_precision_loss)]
fn scatter(chart: &Chart) -> Drawing {
    let xs: Vec<f64> = chart
        .series
        .iter()
        .flat_map(|s| s.xs.iter().copied())
        .collect();
    let ys: Vec<f64> = chart
        .series
        .iter()
        .flat_map(|s| s.values.iter().copied())
        .collect();
    let fold = |v: &[f64]| {
        (
            v.iter().copied().fold(f64::MAX, f64::min),
            v.iter().copied().fold(f64::MIN, f64::max),
        )
    };
    let (xmin, xmax) = if xs.is_empty() {
        (0.0, ys.len() as f64)
    } else {
        fold(&xs)
    };
    let (ymin, ymax) = fold(&ys);
    let (xlo, xhi, xstep) = nice_axis(xmin.min(0.0), xmax);
    let (ylo, yhi, ystep) = nice_axis(
        chart.ymin.unwrap_or_else(|| ymin.min(0.0)),
        chart.ymax.unwrap_or(ymax),
    );
    let legend_h = if chart.legend { 24.0 } else { 0.0 };
    let height = TOP + PLOT_H + AXIS + legend_h;
    let title = if chart.title.is_empty() {
        "Chart"
    } else {
        chart.title.as_str()
    };
    let mut svg = Svg::new(W, height, title);
    let plot_w = W - LEFT - RIGHT;
    let x_of = |v: f64| LEFT + (v - xlo) / (xhi - xlo) * plot_w;
    let y_of = |v: f64| TOP + PLOT_H - (v - ylo) / (yhi - ylo) * PLOT_H;
    let ticks = |lo: f64, hi: f64, step: f64| {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to 0..=100"
        )]
        let n = ((hi - lo) / step).round().clamp(0.0, 100.0) as usize;
        (0..=n).map(move |i| lo + step * i as f64)
    };
    for t in ticks(ylo, yhi, ystep) {
        svg.line(LEFT, y_of(t), W - RIGHT, y_of(t), GRID, 1.0, false);
        svg.text(
            LEFT - 8.0,
            y_of(t) + 4.0,
            &format_value(t, ""),
            11.0,
            MUTED,
            "end",
            false,
        );
    }
    for t in ticks(xlo, xhi, xstep) {
        svg.text(
            x_of(t),
            TOP + PLOT_H + 18.0,
            &format_value(t, ""),
            11.0,
            MUTED,
            "middle",
            false,
        );
    }
    for (si, s) in chart.series.iter().enumerate() {
        for (i, y) in s.values.iter().enumerate() {
            let x = s.xs.get(i).copied().unwrap_or(i as f64);
            let tip = format!(
                "{}, {}",
                format_value(x, ""),
                format_value(*y, &chart.unit)
            );
            svg.circle(x_of(x), y_of(*y), 4.0, &series_colour(si), Some(&tip));
        }
    }
    if chart.legend {
        legend(&mut svg, chart, height - 8.0);
    }
    svg.finish(alt_text(chart))
}

fn radial(chart: &Chart) -> Drawing {
    let values: Vec<f64> = chart
        .series
        .first()
        .map(|s| s.values.iter().map(|v| v.max(0.0)).collect())
        .unwrap_or_default();
    let total: f64 = values.iter().sum::<f64>().max(f64::MIN_POSITIVE);
    let rows = values.len();
    #[allow(clippy::cast_precision_loss)]
    let height = (240.0_f64).max(24.0 + rows as f64 * 22.0);
    let title = if chart.title.is_empty() {
        "Chart"
    } else {
        chart.title.as_str()
    };
    let mut svg = Svg::new(W, height, title);
    let (cx, cy, r) = (130.0, height / 2.0, 100.0);
    let inner = if chart.kind == ChartKind::Donut {
        r * 0.6
    } else {
        0.0
    };
    let mut a0 = -PI / 2.0;
    for (i, v) in values.iter().enumerate() {
        let label = chart.labels.get(i).cloned().unwrap_or_default();
        let share = v / total;
        let tip = format!(
            "{label}: {} ({}%)",
            format_value(*v, &chart.unit),
            num((share * 1000.0).round() / 10.0)
        );
        if share >= 0.999_9 {
            svg.circle(cx, cy, r, &series_colour(i), Some(&tip));
        } else if share > 0.0 {
            let a1 = a0 + share * TAU;
            let large = u8::from(a1 - a0 > PI);
            let (x0, y0) = (cx + r * a0.cos(), cy + r * a0.sin());
            let (x1, y1) = (cx + r * a1.cos(), cy + r * a1.sin());
            let d = if inner > 0.0 {
                let (ix1, iy1) = (cx + inner * a1.cos(), cy + inner * a1.sin());
                let (ix0, iy0) = (cx + inner * a0.cos(), cy + inner * a0.sin());
                format!(
                    "M{} {}A{} {} 0 {large} 1 {} {}L{} {}A{} {} 0 {large} 0 {} \
                        {}Z",
                    num(x0),
                    num(y0),
                    num(r),
                    num(r),
                    num(x1),
                    num(y1),
                    num(ix1),
                    num(iy1),
                    num(inner),
                    num(inner),
                    num(ix0),
                    num(iy0)
                )
            } else {
                format!(
                    "M{} {}L{} {}A{} {} 0 {large} 1 {} {}Z",
                    num(cx),
                    num(cy),
                    num(x0),
                    num(y0),
                    num(r),
                    num(r),
                    num(x1),
                    num(y1)
                )
            };
            svg.path(
                &d,
                &series_colour(i),
                "var(--vl-surface)",
                1.5,
                Some(&tip),
            );
            a0 = a1;
        }
        // Legend row.
        #[allow(clippy::cast_precision_loss)]
        let y =
            24.0 + i as f64 * 22.0 + (height - 24.0 - rows as f64 * 22.0) / 2.0;
        svg.rect(270.0, y - 10.0, 12.0, 12.0, 3.0, &series_colour(i), None);
        let text = format!(
            "{} · {}%",
            fit(&label, 220.0, 13.0),
            num((share * 1000.0).round() / 10.0)
        );
        svg.text(290.0, y, &text, 13.0, INK, "start", false);
    }
    if inner > 0.0 {
        svg.text(
            cx,
            cy + 6.0,
            &format_value(total, &chart.unit),
            18.0,
            INK,
            "middle",
            true,
        );
    }
    svg.finish(alt_text(chart))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model;

    fn chart(src: &str) -> Chart {
        let vellum_format::blocks::Data::Chart(c) =
            model::fence_data("chart", src)
        else {
            panic!()
        };
        model::chart(&c).unwrap()
    }

    #[test]
    fn formats_values() {
        assert_eq!(format_value(1234.5, ""), "1,234.5");
        assert_eq!(format_value(-1_234_567.0, "ms"), "-1,234,567 ms");
        assert_eq!(format_value(40.0, "%"), "40%");
    }

    #[test]
    fn nice_axes() {
        assert_eq!(nice_axis(0.0, 240.0), (0.0, 250.0, 50.0));
        assert_eq!(nice_axis(0.0, 0.0), (-1.0, 1.0, 0.5));
    }

    #[test]
    fn draws_every_kind() {
        for kind in ["bar", "line", "area", "grouped", "stacked"] {
            let d = draw(&chart(&format!(
                "title: T\ntype: {kind}\nlabels: [A, B]\nOne: [1, 2]\n\
                    Two: [3, -1]\n"
            )));
            assert!(d.svg.starts_with("<svg"), "{kind}");
            assert!(d.svg.contains("var(--vl-chart-2)"), "{kind}");
            assert!(d.alt.contains("One: A 1, B 2."), "{kind}: {}", d.alt);
        }
        let d = draw(&chart("title: Pie\ntype: donut\nA: 60\nB: 40\n"));
        assert!(d.svg.contains("A: 60 (60%)"), "{}", d.svg);
        let d = draw(&chart(
            "title: S\ntype: scatter\nlabels: [1, 3]\nvalues: [2, 4]\n",
        ));
        assert_eq!(d.svg.matches("<circle").count(), 2);
    }

    #[test]
    fn output_sanitises_cleanly() {
        let d = draw(&chart(
            "title: \"<script>\"\ntype: bar\nlabels: [\"<b>\"]\nvalues: [1]\n",
        ));
        let clean =
            crate::sanitize::xml::sanitize_island(&d.svg, "f0").unwrap();
        assert!(!clean.contains("<script"));
        assert!(clean.contains("&lt;b&gt;"));
        assert!(clean.contains("aria-labelledby=\"f0-title\""), "{clean}");
    }
}
