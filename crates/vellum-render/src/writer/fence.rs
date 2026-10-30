// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Fenced blocks: code, data fences and diagrams.

use vellum_format::{Fence, FenceKind};

use super::Writer;
use super::inline::safe_image;
use crate::code::{self, CodeOptions};
use crate::figure::{Figure, FigureError};
use crate::html::{self, num, push_attr};
use crate::model;
use vellum_format::blocks::{
    Board, Calculator, CollectionView, Data, DataTable, Image, Timeline,
};

fn pair<'a>(f: &'a Fence, key: &str) -> Option<&'a str> {
    f.attrs
        .pairs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

impl Writer<'_> {
    pub(super) fn fence(&mut self, f: &Fence) {
        match f.kind() {
            FenceKind::Code => self.code(f),
            FenceKind::Chart
                if pair(f, "source").is_some()
                    && f.literal.trim().is_empty() =>
            {
                self.live_chart(f);
            }
            FenceKind::Chart
            | FenceKind::Roadmap
            | FenceKind::RiskMatrix
            | FenceKind::Architecture
            | FenceKind::Diagram => {
                self.figure_block(f);
            }
            FenceKind::Timeline
            | FenceKind::Board
            | FenceKind::Table
            | FenceKind::Calculator
            | FenceKind::Image
            | FenceKind::Collection => match self.data(Data::from_fence(f)) {
                Some(Data::Timeline(t)) => self.timeline(f, &t),
                Some(Data::Board(b)) => self.board(f, &b),
                Some(Data::Table(t)) => self.data_table(f, &t),
                Some(Data::Calculator(c)) => self.calculator(f, &c),
                Some(Data::Image(i)) => self.image(f, &i),
                Some(Data::Collection(c)) => self.collection(f, &c),
                _ => self.source(f),
            },
        }
    }

    /// The drawing view of a chart fence, if it has values.
    fn chart_view(f: &Fence) -> Option<model::Chart> {
        match Data::from_fence(f)?.data {
            Data::Chart(c) => model::chart(&c).ok(),
            _ => None,
        }
    }

    fn code(&mut self, f: &Fence) {
        let options = CodeOptions {
            numbered: pair(f, "lines").is_some_and(|v| v != "false")
                || f.attrs.classes.iter().any(|c| c == "lines"),
            highlighted: pair(f, "hl")
                .map(code::parse_line_list)
                .unwrap_or_default(),
        };
        self.out.push_str("<figure");
        self.attrs("vl-code", &f.attrs);
        self.out.push('>');
        if let Some(title) = pair(f, "title") {
            self.out.push_str("<figcaption>");
            html::text(&mut self.out, title);
            self.out.push_str("</figcaption>");
        }
        self.out.push_str("<pre");
        if !f.lang.is_empty() {
            push_attr(&mut self.out, "data-lang", &f.lang);
        }
        if options.numbered {
            self.out.push_str(" class=\"vl-numbered\"");
        }
        self.out.push_str("><code>");
        code::highlight(&mut self.out, &f.lang, &f.literal, &options);
        self.out.push_str("</code></pre></figure>");
    }

    /// The fence's source as a readable fallback.
    fn source(&mut self, f: &Fence) {
        self.out.push_str("<pre");
        push_attr(&mut self.out, "data-lang", &f.lang);
        self.out.push_str("><code>");
        html::text(&mut self.out, f.literal.trim_end());
        self.out.push_str("</code></pre>");
    }

    fn figure_block(&mut self, f: &Fence) {
        let kind = match f.kind() {
            FenceKind::Chart => "vl-chart",
            FenceKind::Roadmap => "vl-roadmap",
            FenceKind::RiskMatrix => "vl-risk",
            FenceKind::Architecture => "vl-architecture",
            _ => match crate::figure::engine(f) {
                Some(crate::figure::Engine::Kroki(_)) => "vl-diagram vl-kroki",
                _ => "vl-diagram",
            },
        };
        let result = self.figure(f).unwrap_or(Err(FigureError::Unsupported));
        self.out.push_str("<figure");
        self.attrs(&format!("vl-figure {kind}"), &f.attrs);
        push_attr(&mut self.out, "data-kind", &f.lang);
        self.out.push('>');
        let caption = pair(f, "title").map(str::to_owned).or_else(|| {
            (f.kind() == FenceKind::Chart)
                .then(|| Self::chart_view(f).map(|c| c.title))
                .flatten()
                .filter(|t| !t.is_empty())
        });
        if let Some(caption) = &caption {
            self.out.push_str("<figcaption>");
            html::text(&mut self.out, caption);
            self.out.push_str("</figcaption>");
        }
        match result {
            Ok(figure) => self.figure_ok(f, &figure),
            Err(e) => self.figure_err(f, &e),
        }
        self.out.push_str("</figure>");
    }

    fn figure_ok(&mut self, f: &Fence, figure: &Figure) {
        if let Err(e) = self.island(&figure.svg) {
            return self.figure_err(
                f,
                &FigureError::Source {
                    message: format!("invalid SVG: {e}"),
                },
            );
        }
        // The data behind a chart, for screen readers.
        if f.kind() == FenceKind::Chart {
            if let Some(chart) = Self::chart_view(f) {
                self.chart_table(&chart);
            }
        } else {
            self.out.push_str("<p class=\"vl-sr-only\">");
            html::text(&mut self.out, &figure.alt);
            self.out.push_str("</p>");
        }
        if figure.stale {
            self.stale += 1;
            self.out.push_str(
                "<details class=\"vl-figure-source\" \
                data-stale=\"true\"><summary>Showing the last rendered \
                version: the diagram service is unavailable.</summary>",
            );
            self.source(f);
            self.out.push_str("</details>");
        }
    }

    fn figure_err(&mut self, f: &Fence, e: &FigureError) {
        let message = match e {
            FigureError::Unsupported => {
                "This block cannot be drawn; showing its source.".to_owned()
            }
            FigureError::Source { message } => {
                format!("This {} block has a problem: {message}", f.lang)
            }
            FigureError::Unavailable { message } => {
                format!("Showing the source because {message}.")
            }
        };
        self.warn(message.clone());
        self.out
            .push_str("<p class=\"vl-figure-notice\" role=\"status\">");
        html::text(&mut self.out, &message);
        self.out.push_str("</p>");
        self.source(f);
    }

    fn chart_table(&mut self, chart: &model::Chart) {
        self.out.push_str(
            "<table class=\"vl-sr-only\"><thead><tr><th scope=\"col\">",
        );
        html::text(
            &mut self.out,
            if chart.kind == model::ChartKind::Scatter {
                "x"
            } else {
                "Label"
            },
        );
        self.out.push_str("</th>");
        for s in &chart.series {
            self.out.push_str("<th scope=\"col\">");
            html::text(
                &mut self.out,
                if s.name.is_empty() { "Value" } else { &s.name },
            );
            self.out.push_str("</th>");
        }
        self.out.push_str("</tr></thead><tbody>");
        let rows = chart
            .series
            .iter()
            .map(|s| s.values.len())
            .max()
            .unwrap_or(0);
        for i in 0..rows {
            self.out.push_str("<tr><th scope=\"row\">");
            let label = if chart.kind == model::ChartKind::Scatter {
                chart
                    .series
                    .first()
                    .and_then(|s| s.xs.get(i))
                    .map(|x| num(*x))
                    .unwrap_or_default()
            } else {
                chart
                    .labels
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| (i + 1).to_string())
            };
            html::text(&mut self.out, &label);
            self.out.push_str("</th>");
            for s in &chart.series {
                self.out.push_str("<td>");
                if let Some(v) = s.values.get(i) {
                    html::text(
                        &mut self.out,
                        &crate::figure::chart::format_value(*v, &chart.unit),
                    );
                }
                self.out.push_str("</td>");
            }
            self.out.push_str("</tr>");
        }
        self.out.push_str("</tbody></table>");
    }

    fn live_chart(&mut self, f: &Fence) {
        self.out.push_str("<figure");
        self.attrs("vl-figure vl-chart vl-live-placeholder", &f.attrs);
        for (key, attr) in
            [("source", "data-source"), ("x", "data-x"), ("y", "data-y")]
        {
            if let Some(v) = pair(f, key) {
                push_attr(&mut self.out, attr, v);
            }
        }
        self.out.push_str(
            ">A live chart of a collection; it fills in when the \
                page opens.</figure>",
        );
    }

    fn timeline(&mut self, f: &Fence, t: &Timeline) {
        self.out.push_str("<ol");
        self.attrs("vl-timeline", &f.attrs);
        self.out.push('>');
        for item in &t.items {
            self.out.push_str("<li><time");
            if model::days_from_iso(&item.when).is_some() {
                push_attr(&mut self.out, "datetime", &item.when);
            }
            self.out.push('>');
            html::text(&mut self.out, &item.when);
            self.out.push_str("</time><strong>");
            html::text(&mut self.out, &item.title);
            self.out.push_str("</strong>");
            if let Some(detail) = &item.detail {
                self.out.push_str("<div class=\"vl-timeline-detail\">");
                html::text(&mut self.out, detail);
                self.out.push_str("</div>");
            }
            self.out.push_str("</li>");
        }
        self.out.push_str("</ol>");
    }

    fn board(&mut self, f: &Fence, board: &Board) {
        if board.columns.is_empty() {
            self.warn("board needs `## Column` headings and `- card` items");
            return self.source(f);
        }
        self.out.push_str("<div");
        self.attrs("vl-board", &f.attrs);
        self.out.push('>');
        for col in &board.columns {
            self.out
                .push_str("<section class=\"vl-board-column\"><h4><span>");
            html::text(&mut self.out, &col.name);
            self.out.push_str("</span><span class=\"vl-board-count\">");
            self.out.push_str(&col.cards.len().to_string());
            self.out.push_str("</span></h4><ul>");
            for card in &col.cards {
                self.out.push_str("<li class=\"vl-board-card\">");
                html::text(&mut self.out, &card.text);
                if let Some(tag) = &card.tag {
                    let class = match tag.to_ascii_uppercase().as_str() {
                        "P0" => "vl-chip vl-chip-crit",
                        "P1" | "P2" => "vl-chip vl-chip-warn",
                        _ => "vl-chip",
                    };
                    self.out.push_str(" <span");
                    push_attr(&mut self.out, "class", class);
                    self.out.push('>');
                    html::text(&mut self.out, tag);
                    self.out.push_str("</span>");
                }
                self.out.push_str("</li>");
            }
            self.out.push_str("</ul></section>");
        }
        self.out.push_str("</div>");
    }

    fn data_table(&mut self, f: &Fence, table: &DataTable) {
        if table.header.is_empty() {
            self.warn("table needs a CSV header row");
            return self.source(f);
        }
        let col = |name: Option<&str>| {
            name.and_then(|n| {
                table.header.iter().position(|h| h.eq_ignore_ascii_case(n))
            })
        };
        let bar = col(table.bar.as_deref());
        let sums: Vec<usize> = table
            .sum
            .iter()
            .filter_map(|n| col(Some(n.trim())))
            .collect();
        let sortable = table.sortable;
        let bar_max = bar.map_or(0.0, |b| {
            table
                .rows
                .iter()
                .filter_map(|r| r.get(b).and_then(|v| model::parse_number(v)))
                .fold(0.0, f64::max)
        });
        self.out.push_str("<div class=\"vl-table-wrap\"><table");
        self.attrs("vl-datatable", &f.attrs);
        if sortable {
            self.out.push_str(" data-sortable=\"true\"");
        }
        self.out.push_str("><thead><tr>");
        for h in &table.header {
            self.out.push_str("<th scope=\"col\"");
            if sortable {
                self.out.push_str(" aria-sort=\"none\"");
            }
            self.out.push('>');
            html::text(&mut self.out, h);
            self.out.push_str("</th>");
        }
        self.out.push_str("</tr></thead><tbody>");
        for row in &table.rows {
            self.out.push_str("<tr>");
            for (i, cell) in row.iter().enumerate() {
                let numeric = model::parse_number(cell).is_some();
                self.out.push_str(if numeric {
                    "<td class=\"vl-align-right\">"
                } else {
                    "<td>"
                });
                match (Some(i) == bar, model::parse_number(cell)) {
                    (true, Some(v)) => {
                        self.out.push_str(
                            "<span class=\"vl-bar-cell\"><meter min=\"0\"",
                        );
                        push_attr(
                            &mut self.out,
                            "max",
                            &num(bar_max.max(f64::MIN_POSITIVE)),
                        );
                        push_attr(&mut self.out, "value", &num(v));
                        self.out.push_str("></meter><span>");
                        html::text(&mut self.out, cell);
                        self.out.push_str("</span></span>");
                    }
                    _ => html::text(&mut self.out, cell),
                }
                self.out.push_str("</td>");
            }
            self.out.push_str("</tr>");
        }
        self.out.push_str("</tbody>");
        if !sums.is_empty() {
            self.out.push_str("<tfoot><tr>");
            for i in 0..table.header.len() {
                if sums.contains(&i) {
                    let total: f64 = table
                        .rows
                        .iter()
                        .filter_map(|r| {
                            r.get(i).and_then(|v| model::parse_number(v))
                        })
                        .sum();
                    self.out.push_str("<td class=\"vl-align-right\">");
                    html::text(
                        &mut self.out,
                        &crate::figure::chart::format_value(total, ""),
                    );
                } else if i == 0 {
                    self.out.push_str("<td>Total");
                } else {
                    self.out.push_str("<td>");
                }
                self.out.push_str("</td>");
            }
            self.out.push_str("</tr></tfoot>");
        }
        self.out.push_str("</table></div>");
    }

    fn calculator(&mut self, f: &Fence, calc: &Calculator) {
        if calc.outputs.is_empty() {
            self.warn(
                "calculator needs a result line such as `total: Total = \
                units * price`",
            );
            return self.source(f);
        }
        let mut vars: Vec<(String, f64)> = calc
            .inputs
            .iter()
            .map(|i| (i.key.clone(), i.value))
            .collect();
        self.out.push_str("<div");
        self.attrs("vl-calculator", &f.attrs);
        self.out.push('>');
        for input in &calc.inputs {
            let id =
                self.unique_id(&format!("calc-{}", html::slug(&input.key)));
            let min = input.min.unwrap_or_else(|| input.value.min(0.0));
            let max = input
                .max
                .unwrap_or_else(|| (input.value * 2.0).max(min + 1.0));
            let step = input.step.filter(|s| *s > 0.0).unwrap_or(1.0);
            self.out.push_str("<label");
            push_attr(&mut self.out, "for", &id);
            self.out.push_str("><span>");
            html::text(&mut self.out, &input.label);
            self.out.push_str("</span><input type=\"range\"");
            push_attr(&mut self.out, "id", &id);
            push_attr(&mut self.out, "data-key", &input.key);
            push_attr(&mut self.out, "min", &num(min));
            push_attr(&mut self.out, "max", &num(max));
            push_attr(&mut self.out, "step", &num(step));
            push_attr(&mut self.out, "value", &num(input.value));
            self.out.push_str("><output");
            push_attr(&mut self.out, "for", &id);
            self.out.push('>');
            html::text(&mut self.out, &num(input.value));
            self.out.push_str("</output></label>");
        }
        for output in &calc.outputs {
            let unit = output.unit.as_deref().unwrap_or_default();
            let result = model::evaluate(&output.formula, &vars);
            self.out.push_str("<div class=\"vl-calc-result\"");
            push_attr(&mut self.out, "data-key", &output.key);
            push_attr(&mut self.out, "data-formula", &output.formula);
            push_attr(&mut self.out, "data-unit", unit);
            self.out.push_str("><span>");
            html::text(&mut self.out, &output.label);
            self.out.push_str("</span> <output>");
            match result {
                Ok(v) => {
                    html::text(
                        &mut self.out,
                        &crate::figure::chart::format_value(v, unit),
                    );
                    // Later outputs may use earlier ones.
                    vars.push((output.key.clone(), v));
                }
                Err(e) => {
                    self.warn(format!("calculator `{}`: {e}", output.key));
                    html::text(&mut self.out, "–");
                }
            }
            self.out.push_str("</output></div>");
        }
        self.out.push_str("</div>");
    }

    fn image(&mut self, f: &Fence, image: &Image) {
        let Some(src) = safe_image(&image.src) else {
            self.warn("image needs `src:` with an https or relative URL");
            return self.source(f);
        };
        let align = image
            .align
            .as_deref()
            .filter(|a| matches!(*a, "left" | "right" | "center"));
        let class = align.map_or_else(
            || "vl-figure vl-image".to_owned(),
            |a| format!("vl-figure vl-image vl-align-{a}"),
        );
        self.out.push_str("<figure");
        self.attrs(&class, &f.attrs);
        self.out.push_str("><img");
        push_attr(&mut self.out, "src", &src);
        push_attr(&mut self.out, "alt", &image.alt);
        if let Some(w) = image.width.as_deref().filter(|w| {
            w.trim_end_matches(['%', 'x', 'p']).parse::<u32>().is_ok()
        }) {
            push_attr(&mut self.out, "width", w.trim_end_matches("px"));
        }
        self.out.push_str(" loading=\"lazy\" decoding=\"async\">");
        if let Some(caption) = &image.caption {
            self.out.push_str("<figcaption>");
            html::text(&mut self.out, caption);
            self.out.push_str("</figcaption>");
        }
        self.out.push_str("</figure>");
    }

    fn collection(&mut self, f: &Fence, c: &CollectionView) {
        let (id, view, filter) = (
            &c.id,
            c.view.clone().unwrap_or_default(),
            c.filter.clone().unwrap_or_default(),
        );
        self.out.push_str("<div");
        self.attrs("vl-collection", &f.attrs);
        push_attr(&mut self.out, "data-collection", id);
        push_attr(&mut self.out, "data-view", &view);
        if !filter.is_empty() {
            push_attr(&mut self.out, "data-filter", &filter);
        }
        self.out.push('>');
        match self.resolver().collection(id, &view) {
            Some(snap) => {
                if !snap.title.is_empty() {
                    self.out.push_str("<p><strong>");
                    html::text(&mut self.out, &snap.title);
                    self.out.push_str("</strong></p>");
                }
                self.out.push_str(
                    "<div class=\"vl-table-wrap\"><table><thead><tr>",
                );
                for h in &snap.header {
                    self.out.push_str("<th scope=\"col\">");
                    html::text(&mut self.out, h);
                    self.out.push_str("</th>");
                }
                self.out.push_str("</tr></thead><tbody>");
                for row in &snap.rows {
                    self.out.push_str("<tr>");
                    for cell in row {
                        self.out.push_str("<td>");
                        html::text(&mut self.out, cell);
                        self.out.push_str("</td>");
                    }
                    self.out.push_str("</tr>");
                }
                self.out.push_str("</tbody></table></div>");
            }
            None => {
                self.out.push_str("A live view of a collection");
                if !view.is_empty() {
                    self.out.push_str(" (");
                    html::text(&mut self.out, &view);
                    self.out.push(')');
                }
                self.out.push_str("; it fills in when the page opens.");
            }
        }
        self.out.push_str("</div>");
    }
}
