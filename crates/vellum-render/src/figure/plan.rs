// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Plans and structures: `roadmap`, `risk-matrix`, `timeline`, `board` and
//! `architecture` drawings.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::svg::{CARD, Drawing, GRID, INK, MUTED, OUTLINE, Svg, fit, series_colour, text_width};
use crate::html::num;
use vellum_format::blocks::{Architecture, BoardColumn, TimelineItem};

use crate::model::{Risk, Roadmap, days_from_iso, year_month};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn month_name(m: i64) -> &'static str {
    usize::try_from(m - 1)
        .ok()
        .and_then(|i| MONTHS.get(i))
        .copied()
        .unwrap_or("")
}

/// Draws a roadmap as a Gantt chart with month ticks.
///
/// The root carries `data-vl-range="start-day end-day x0 x1"` so the client
/// can draw a live "today" line without re-rendering the page.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn roadmap(r: &Roadmap) -> Drawing {
    let label_w = r
        .rows
        .iter()
        .map(|row| text_width(&row.name, 13.0))
        .fold(110.0, f64::max)
        .min(240.0)
        + 16.0;
    let width = 720.0;
    let (top, row_h) = (36.0, 30.0);
    let height = top + row_h * r.rows.len() as f64 + 12.0;
    let plot = width - label_w - 16.0;
    let span = (r.end - r.start).max(1) as f64;
    let x_of = |d: i64| label_w + (d - r.start) as f64 / span * plot;
    let mut svg = Svg::new(width, height, "Roadmap");
    svg.raw(&format!(
        "<g data-vl-range=\"{} {} {} {}\"></g>",
        r.start,
        r.end,
        num(label_w),
        num(label_w + plot)
    ));
    // Month ticks, thinned so labels never overlap.
    let (mut y, mut m) = year_month(r.start);
    let months = {
        let (ey, em) = year_month(r.end);
        (ey - y) * 12 + (em - m) + 1
    };
    let every = [1, 2, 3, 6, 12]
        .into_iter()
        .find(|s| plot / months.max(1) as f64 * f64::from(*s) >= 48.0)
        .unwrap_or(12);
    let mut i = 0;
    loop {
        let first = days_from_iso(&format!("{y:04}-{m:02}-01")).unwrap_or(r.start);
        if first > r.end {
            break;
        }
        if first >= r.start && i % every == 0 {
            let x = x_of(first);
            svg.line(x, top - 8.0, x, height - 8.0, GRID, 1.0, false);
            let label = if m == 1 || i == 0 {
                format!("{} {y}", month_name(m))
            } else {
                month_name(m).to_owned()
            };
            svg.text(x + 3.0, top - 14.0, &label, 11.0, MUTED, "start", false);
        }
        i += 1;
        m += 1;
        if m > 12 {
            m = 1;
            y += 1;
        }
        if i > 600 {
            break;
        }
    }
    for (idx, row) in r.rows.iter().enumerate() {
        let cy = top + row_h * idx as f64 + row_h / 2.0;
        svg.text(
            8.0,
            cy + 4.0,
            &fit(&row.name, label_w - 16.0, 13.0),
            13.0,
            INK,
            "start",
            false,
        );
        let colour = series_colour(idx);
        if row.milestone {
            let x = x_of(row.start);
            let d = format!(
                "M{} {}L{} {}L{} {}L{} {}Z",
                num(x),
                num(cy - 8.0),
                num(x + 8.0),
                num(cy),
                num(x),
                num(cy + 8.0),
                num(x - 8.0),
                num(cy)
            );
            svg.path(
                &d,
                &colour,
                "none",
                0.0,
                Some(&format!("{} · milestone", row.name)),
            );
        } else {
            let x0 = x_of(row.start);
            let x1 = x_of(row.end).max(x0 + 4.0);
            svg.rect(x0, cy - 9.0, x1 - x0, 18.0, 6.0, &colour, Some(&row.name));
        }
    }
    let alt = format!(
        "Roadmap with {} rows: {}.",
        r.rows.len(),
        r.rows
            .iter()
            .map(|row| row.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    svg.finish(alt)
}

/// The colour token for a risk score (likelihood × impact).
const fn risk_fill(score: u8) -> &'static str {
    match score {
        15.. => "var(--vl-crit-container)",
        8..=14 => "var(--vl-warn-container)",
        4..=7 => "var(--vl-highlight)",
        _ => "var(--vl-good-container)",
    }
}

/// Draws a 5×5 likelihood/impact matrix with numbered markers.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn risk_matrix(risks: &[Risk]) -> Drawing {
    let (left, top, cw, ch) = (64.0, 12.0, 96.0, 52.0);
    let width = left + cw * 5.0 + 16.0;
    let height = top + ch * 5.0 + 44.0;
    let mut svg = Svg::new(width, height, "Risk matrix");
    for likelihood in 1..=5u8 {
        for impact in 1..=5u8 {
            let x = left + f64::from(impact - 1) * cw;
            let y = top + f64::from(5 - likelihood) * ch;
            svg.boxed(
                x + 1.0,
                y + 1.0,
                cw - 2.0,
                ch - 2.0,
                6.0,
                risk_fill(likelihood * impact),
                "var(--vl-surface)",
            );
        }
        let y = top + f64::from(5 - likelihood) * ch + ch / 2.0 + 4.0;
        svg.text(
            left - 10.0,
            y,
            &likelihood.to_string(),
            12.0,
            MUTED,
            "end",
            false,
        );
    }
    for impact in 1..=5u8 {
        let x = left + f64::from(impact - 1) * cw + cw / 2.0;
        svg.text(
            x,
            top + ch * 5.0 + 16.0,
            &impact.to_string(),
            12.0,
            MUTED,
            "middle",
            false,
        );
    }
    svg.text(
        left + cw * 2.5,
        height - 6.0,
        "Impact",
        12.0,
        MUTED,
        "middle",
        true,
    );
    svg.raw(&format!(
        "<text x=\"14\" y=\"{}\" font-size=\"12\" font-weight=\"600\" text-anchor=\"middle\" transform=\"rotate(-90 14 {})\" style=\"fill:{MUTED}\">Likelihood</text>",
        num(top + ch * 2.5),
        num(top + ch * 2.5)
    ));
    // Markers, spread within their cell.
    let mut per_cell: HashMap<(u8, u8), usize> = HashMap::new();
    for (n, risk) in risks.iter().enumerate() {
        let slot = per_cell.entry((risk.likelihood, risk.impact)).or_insert(0);
        let cx = left + f64::from(risk.impact - 1) * cw + 18.0 + (*slot % 4) as f64 * 22.0;
        let cy = top + f64::from(5 - risk.likelihood) * ch + 16.0 + (*slot / 4) as f64 * 20.0;
        *slot += 1;
        let tip = format!(
            "{}. {} (likelihood {}, impact {})",
            n + 1,
            risk.title,
            risk.likelihood,
            risk.impact
        );
        svg.circle(cx, cy, 9.0, INK, Some(&tip));
        svg.text(
            cx,
            cy + 4.0,
            &(n + 1).to_string(),
            11.0,
            "var(--vl-surface)",
            "middle",
            true,
        );
    }
    let mut alt = format!("Risk matrix with {} risks.", risks.len());
    for (n, r) in risks.iter().enumerate() {
        let _ = write!(
            alt,
            " {}. {}: likelihood {}, impact {}.",
            n + 1,
            r.title,
            r.likelihood,
            r.impact
        );
    }
    svg.finish(alt)
}

/// Draws a vertical timeline (for exports; pages use an HTML list).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn timeline(items: &[TimelineItem]) -> Drawing {
    let row = 44.0;
    let height = 16.0 + row * items.len().max(1) as f64;
    let mut svg = Svg::new(640.0, height, "Timeline");
    svg.line(100.0, 12.0, 100.0, height - 12.0, GRID, 2.0, false);
    for (i, item) in items.iter().enumerate() {
        let y = 24.0 + row * i as f64;
        svg.text(
            88.0,
            y + 4.0,
            &fit(&item.when, 84.0, 12.0),
            12.0,
            MUTED,
            "end",
            false,
        );
        svg.circle(100.0, y, 6.0, "var(--vl-primary)", None);
        svg.text(
            116.0,
            y + 4.0,
            &fit(&item.title, 500.0, 14.0),
            14.0,
            INK,
            "start",
            true,
        );
        if let Some(detail) = &item.detail {
            svg.text(
                116.0,
                y + 20.0,
                &fit(detail, 500.0, 12.0),
                12.0,
                MUTED,
                "start",
                false,
            );
        }
    }
    let alt = items
        .iter()
        .map(|i| format!("{}: {}", i.when, i.title))
        .collect::<Vec<_>>()
        .join("; ");
    svg.finish(format!("Timeline. {alt}."))
}

/// Draws a board's columns and cards (for exports; pages use HTML).
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn board(columns: &[BoardColumn]) -> Drawing {
    let col_w = 180.0;
    let gap = 12.0;
    let tallest = columns.iter().map(|c| c.cards.len()).max().unwrap_or(0) as f64;
    let width = (col_w + gap) * columns.len().max(1) as f64 + gap;
    let height = 48.0 + tallest * 52.0 + 12.0;
    let mut svg = Svg::new(width, height, "Board");
    for (i, col) in columns.iter().enumerate() {
        let x = gap + (col_w + gap) * i as f64;
        svg.boxed(x, 8.0, col_w, height - 16.0, 12.0, CARD, "none");
        svg.text(
            x + 12.0,
            30.0,
            &fit(&col.name, col_w - 48.0, 13.0),
            13.0,
            INK,
            "start",
            true,
        );
        svg.text(
            x + col_w - 12.0,
            30.0,
            &col.cards.len().to_string(),
            12.0,
            MUTED,
            "end",
            false,
        );
        for (j, card) in col.cards.iter().enumerate() {
            let y = 44.0 + 52.0 * j as f64;
            svg.boxed(
                x + 8.0,
                y,
                col_w - 16.0,
                44.0,
                8.0,
                "var(--vl-surface-container-lowest)",
                GRID,
            );
            svg.text(
                x + 16.0,
                y + 18.0,
                &fit(&card.text, col_w - 32.0, 12.0),
                12.0,
                INK,
                "start",
                false,
            );
            if let Some(tag) = &card.tag {
                svg.text(x + 16.0, y + 35.0, tag, 11.0, MUTED, "start", true);
            }
        }
    }
    let alt = columns
        .iter()
        .map(|c| format!("{} ({} cards)", c.name, c.cards.len()))
        .collect::<Vec<_>>()
        .join(", ");
    svg.finish(format!("Board: {alt}."))
}

/// Draws lanes of boxes with arrows between them. Lanes are rows, items are
/// laid out left to right; arrows run centre to centre between box edges.
#[must_use]
#[allow(clippy::cast_precision_loss)]
pub fn architecture(a: &Architecture) -> Drawing {
    let (lane_label, box_h, lane_h, gap) = (120.0, 36.0, 66.0, 16.0);
    let box_w = |t: &str| (text_width(t, 13.0) + 24.0).clamp(96.0, 220.0);
    let width = a
        .lanes
        .iter()
        .map(|lane| lane_label + 16.0 + lane.items.iter().map(|i| box_w(i) + gap).sum::<f64>())
        .fold(640.0, f64::max);
    let height = 12.0 + (lane_h + 10.0) * a.lanes.len().max(1) as f64;
    let mut svg = Svg::new(width, height, "Architecture diagram");
    svg.raw("<defs><marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"9\" refY=\"5\" markerWidth=\"7\" markerHeight=\"7\" orient=\"auto-start-reverse\"><path d=\"M0 0L10 5L0 10Z\" style=\"fill:var(--vl-on-surface-variant)\"/></marker></defs>");
    let mut pos: HashMap<&str, (f64, f64, f64)> = HashMap::new();
    for (i, lane) in a.lanes.iter().enumerate() {
        let (name, items) = (&lane.name, &lane.items);
        let y0 = 12.0 + (lane_h + 10.0) * i as f64;
        svg.boxed(4.0, y0, width - 8.0, lane_h, 12.0, CARD, "none");
        svg.text(
            16.0,
            y0 + lane_h / 2.0 + 4.0,
            &fit(name, lane_label - 20.0, 12.0),
            12.0,
            MUTED,
            "start",
            true,
        );
        let mut x = lane_label + 8.0;
        for item in items {
            let w = box_w(item);
            let top = y0 + (lane_h - box_h) / 2.0;
            svg.boxed(
                x,
                top,
                w,
                box_h,
                8.0,
                "var(--vl-surface-container-lowest)",
                OUTLINE,
            );
            svg.text(
                x + w / 2.0,
                top + box_h / 2.0 + 4.0,
                &fit(item, w - 12.0, 13.0),
                13.0,
                INK,
                "middle",
                false,
            );
            let _ = pos.insert(item.as_str(), (x + w / 2.0, top, w));
            x += w + gap;
        }
    }
    for arrow in &a.arrows {
        let (from, to) = (&arrow.from, &arrow.to);
        let (Some(&(ax, at, aw)), Some(&(bx, bt, bw))) =
            (pos.get(from.as_str()), pos.get(to.as_str()))
        else {
            continue;
        };
        let d = if (at - bt).abs() < 1.0 {
            // Same lane: horizontal arrow between facing edges.
            let (l, r) = if ax < bx {
                (ax + aw / 2.0, bx - bw / 2.0)
            } else {
                (ax - aw / 2.0, bx + bw / 2.0)
            };
            let y = at + box_h / 2.0;
            format!("M{} {}L{} {}", num(l), num(y), num(r), num(y))
        } else {
            let (y1, y2) = if at < bt {
                (at + box_h, bt)
            } else {
                (at, bt + box_h)
            };
            let mid = f64::midpoint(y1, y2);
            format!(
                "M{} {}C{} {} {} {} {} {}",
                num(ax),
                num(y1),
                num(ax),
                num(mid),
                num(bx),
                num(mid),
                num(bx),
                num(y2)
            )
        };
        let label = arrow
            .label
            .as_deref()
            .map(|l| format!(": {l}"))
            .unwrap_or_default();
        svg.raw(&format!(
            "<path d=\"{d}\" marker-end=\"url(#arrow)\" style=\"fill:none;stroke:{MUTED};stroke-width:1.5\"><title>{} → {}{}</title></path>",
            crate::html::escape(from),
            crate::html::escape(to),
            crate::html::escape(&label)
        ));
    }
    let mut alt = String::from("Architecture diagram.");
    for lane in &a.lanes {
        let _ = write!(alt, " {}: {}.", lane.name, lane.items.join(", "));
    }
    for arrow in &a.arrows {
        let _ = write!(alt, " {} → {}", arrow.from, arrow.to);
        if let Some(label) = &arrow.label {
            let _ = write!(alt, " ({label})");
        }
        alt.push('.');
    }
    svg.finish(alt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_format::blocks::Data;

    use crate::model;
    use crate::sanitize::xml::sanitize_island;

    #[test]
    fn roadmap_draws_bars_and_milestones() {
        let Data::Roadmap(r) = model::fence_data(
            "roadmap",
            "range: 2026-11-01 .. 2027-06-30\nDiscovery | 2026-11-01 | 2026-12-15\nBuild | 2026-12-01 | 2027-04-30\nLaunch | 2027-05-15 | milestone\n",
        ) else {
            panic!()
        };
        let r = model::roadmap(&r).unwrap();
        let d = roadmap(&r);
        assert!(d.svg.contains("Launch · milestone"));
        assert!(d.svg.contains("Nov 2026"));
        assert!(d.svg.contains("data-vl-range"));
        assert!(sanitize_island(&d.svg, "f").is_ok());
    }

    #[test]
    fn risk_matrix_numbers_risks() {
        let Data::RiskMatrix(m) = model::fence_data(
            "risk-matrix",
            "PDF export slips | likelihood 3 | impact 4\nDrive quota | likelihood 2 | impact 2\n",
        ) else {
            panic!()
        };
        let d = risk_matrix(&m.risks);
        assert!(
            d.svg
                .contains("1. PDF export slips (likelihood 3, impact 4)")
        );
        assert!(d.alt.contains("2. Drive quota"));
        assert!(sanitize_island(&d.svg, "f").is_ok());
    }

    #[test]
    fn architecture_connects_boxes() {
        let Data::Architecture(a) = model::fence_data(
            "architecture",
            "Clients: Web app, Mobile app\nServices: API, Worker\nData: Postgres\nWeb app -> API\nMobile app -> API\nAPI -> Postgres: reads\nAPI -> Worker\n",
        ) else {
            panic!()
        };
        let d = architecture(&a);
        assert_eq!(d.svg.matches("marker-end").count(), 4);
        let clean = sanitize_island(&d.svg, "f9").unwrap();
        assert!(clean.contains("url(#f9-arrow)"), "{clean}");
    }

    #[test]
    fn timeline_and_board_draw() {
        let Data::Timeline(t) = model::fence_data("timeline", "Oct 6 | Design approved | Scope\n")
        else {
            panic!()
        };
        assert!(timeline(&t.items).svg.contains("Design approved"));
        let Data::Board(b) = model::fence_data("board", "## To do\n- First [P1]\n") else {
            panic!()
        };
        assert!(board(&b.columns).svg.contains("P1"));
    }
}
