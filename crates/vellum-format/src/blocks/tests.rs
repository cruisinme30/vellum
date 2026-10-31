// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

use super::*;
use crate::ast::Inline;

/// Reads the typed data of the only block in `src`.
fn read(src: &str) -> Read<Data> {
    let parsed = crate::parse(src);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let [block] = parsed.document.blocks.as_slice() else {
        panic!("expected one block, found {:?}", parsed.document.blocks);
    };
    Data::from_block(block).expect("a data block")
}

fn text(s: &str) -> Vec<Inline> {
    vec![Inline::Text { text: s.to_owned() }]
}

fn lines_of(read: &Read<Data>) -> Vec<usize> {
    read.diagnostics.iter().map(|d| d.line).collect()
}

#[test]
fn code_and_callouts_are_not_data() {
    let parsed =
        crate::parse("```rust\nfn main() {}\n```\n\n::: note\nHi\n:::\n");
    assert!(
        parsed
            .document
            .blocks
            .iter()
            .all(|b| Data::from_block(b).is_none())
    );
}

#[test]
fn chart_with_series_and_options() {
    let read = read(
        "```chart\ntitle: Blocked requests per week\ntype: line\n\
         labels: [W40, W41, W42]\nEMEA: [120, 1_185, 160]\nAPAC: [90, 95, 99]\n\
         unit: req\nymin: 0\nlegend: true\n```\n",
    );
    let Data::Chart(chart) = read.data else {
        panic!()
    };
    assert!(read.diagnostics.is_empty(), "{:?}", read.diagnostics);
    assert_eq!(chart.title, "Blocked requests per week");
    assert_eq!(chart.kind, ChartKind::Line);
    assert_eq!(chart.labels, ["W40", "W41", "W42"]);
    assert_eq!(chart.series.len(), 2);
    assert_eq!(chart.series[0].name, "EMEA");
    assert_eq!(chart.series[0].values, [120.0, 1185.0, 160.0]);
    assert_eq!(chart.unit.as_deref(), Some("req"));
    assert_eq!(chart.ymin, Some(0.0));
    assert_eq!(chart.legend, Some(true));
}

#[test]
fn chart_slices_become_labels() {
    let read = read("```chart\ntype: donut\nSearch: 60\nSync: 40\n```\n");
    let Data::Chart(chart) = read.data else {
        panic!()
    };
    assert_eq!(chart.labels, ["Search", "Sync"]);
    assert_eq!(chart.series[0].values, [60.0, 40.0]);
}

#[test]
fn chart_reports_bad_lines_and_keeps_the_rest() {
    let read = read(
        "```chart\ntype: radar\nlabels: [a, b]\n\
         values: [1, x]\nnonsense\n```\n",
    );
    assert_eq!(lines_of(&read), [1, 3, 4, 0]);
    let Data::Chart(chart) = read.data else {
        panic!()
    };
    assert_eq!(chart.kind, ChartKind::Bar);
    assert_eq!(chart.series[0].values, [1.0]);
}

#[test]
fn live_chart_reads_attributes() {
    let read = read(
        "```chart {source=\"collection:okrs\" \
         x=\"quarter\" y=\"score\"}\n```\n",
    );
    let Data::Chart(chart) = read.data else {
        panic!()
    };
    assert_eq!(chart.source.as_deref(), Some("collection:okrs"));
    assert_eq!(chart.x.as_deref(), Some("quarter"));
    assert_eq!(chart.y.as_deref(), Some("score"));
}

#[test]
fn timeline_entries() {
    let read = read(
        "```timeline\nOct 6 | Beta opens | Invite-only\n\
            2026-11-24 | General availability\nlonely\n```\n",
    );
    assert_eq!(lines_of(&read), [3]);
    let Data::Timeline(timeline) = read.data else {
        panic!()
    };
    assert_eq!(timeline.items.len(), 2);
    assert_eq!(timeline.items[0].detail.as_deref(), Some("Invite-only"));
    assert_eq!(timeline.items[1].when, "2026-11-24");
    assert_eq!(timeline.items[1].detail, None);
}

#[test]
fn roadmap_bars_and_milestones() {
    let read = read(
        "```roadmap\nrange: 2026-10-01 .. 2027-03-31\n\
         Replay engine | 2026-10-01 | 2026-12-15\nGA | 2026-11-24 | milestone\n\
         Later | soon | 2027-01-01\n```\n",
    );
    assert_eq!(lines_of(&read), [4]);
    let Data::Roadmap(roadmap) = read.data else {
        panic!()
    };
    assert_eq!(roadmap.start.as_deref(), Some("2026-10-01"));
    assert_eq!(roadmap.end.as_deref(), Some("2027-03-31"));
    assert_eq!(roadmap.rows.len(), 2);
    assert_eq!(roadmap.rows[0].end.as_deref(), Some("2026-12-15"));
    assert_eq!(roadmap.rows[1].end, None);
}

#[test]
fn risk_matrix_scores() {
    let read = read(
        "```risk-matrix\nData loss | likelihood 1 | impact 5\n\
         Slow search | Impact 2 | likelihood 3\n\
         Unscored | likelihood 9 | impact 1\n```\n",
    );
    assert_eq!(lines_of(&read), [3]);
    let Data::RiskMatrix(matrix) = read.data else {
        panic!()
    };
    assert_eq!(matrix.risks.len(), 2);
    assert_eq!((matrix.risks[1].likelihood, matrix.risks[1].impact), (3, 2));
}

#[test]
fn board_columns_and_tags() {
    let read = read(
        "```board\n- orphan\n## Todo\n- Write runbook [P1]\n## Done\n\
            - Diff report\n```\n",
    );
    assert_eq!(lines_of(&read), [1]);
    let Data::Board(board) = read.data else {
        panic!()
    };
    assert_eq!(board.columns.len(), 2);
    assert_eq!(board.columns[0].cards[0].text, "Write runbook");
    assert_eq!(board.columns[0].cards[0].tag.as_deref(), Some("P1"));
    assert_eq!(board.columns[1].cards[0].tag, None);
}

#[test]
fn table_csv_and_options() {
    let read = read(
        "```table sortable bar=Revenue sum=Revenue\nRegion,Revenue\n\
            \"EMEA, North\",120\nAPAC\n```\n",
    );
    assert_eq!(lines_of(&read), [3]);
    let Data::Table(table) = read.data else {
        panic!()
    };
    assert!(table.sortable);
    assert_eq!(table.bar.as_deref(), Some("Revenue"));
    assert_eq!(table.sum, ["Revenue"]);
    assert_eq!(table.header, ["Region", "Revenue"]);
    assert_eq!(table.rows, [vec!["EMEA, North", "120"], vec!["APAC", ""]]);
}

#[test]
fn architecture_lanes_and_arrows() {
    let read = read(
        "```architecture\nEdge: Gateway\nCore: [Policy engine, Audit log]\n\
         Gateway -> Policy engine: checks\nPolicy engine -> Billing\n```\n",
    );
    assert_eq!(read.diagnostics.len(), 1, "{:?}", read.diagnostics);
    let Data::Architecture(arch) = read.data else {
        panic!()
    };
    assert_eq!(arch.lanes[1].items, ["Policy engine", "Audit log"]);
    assert_eq!(arch.arrows[0].label.as_deref(), Some("checks"));
    assert_eq!(arch.arrows[1].to, "Billing");
}

#[test]
fn calculator_inputs_and_outputs() {
    let read = read(
        "```calculator\nseats: Seats = 40 [1..500 step 1]\nprice: 12\n\
         total: Monthly cost = seats * price ($)\n```\n",
    );
    assert!(read.diagnostics.is_empty(), "{:?}", read.diagnostics);
    let Data::Calculator(calc) = read.data else {
        panic!()
    };
    assert_eq!(calc.inputs.len(), 2);
    assert_eq!(calc.inputs[0].label, "Seats");
    assert_eq!(
        (calc.inputs[0].min, calc.inputs[0].max),
        (Some(1.0), Some(500.0))
    );
    assert_eq!(calc.inputs[1].label, "price");
    assert_eq!(calc.outputs[0].formula, "seats * price");
    assert_eq!(calc.outputs[0].unit.as_deref(), Some("$"));
}

#[test]
fn image_and_collection_keys() {
    let read = self::read(
        "```image\nsrc: /media/map.png\nalt: Region map\nzoom: 2\n```\n",
    );
    assert_eq!(lines_of(&read), [3]);
    let Data::Image(image) = read.data else {
        panic!()
    };
    assert_eq!(image.src, "/media/map.png");

    let read = self::read(
        "```collection\nsource: collection:okrs\nview: By owner\n```\n",
    );
    let Data::Collection(view) = read.data else {
        panic!()
    };
    assert_eq!(view.id, "okrs");
    assert_eq!(view.view.as_deref(), Some("By owner"));

    let read = self::read("```collection\n```\n");
    assert_eq!(lines_of(&read), [0]);
}

#[test]
fn stats_with_trends() {
    let read = read(
        "::: stats\n- 94% · Uptime · ▲ 6 pts\n- 12 · Open incidents\n- lonely\n\
            :::\n",
    );
    assert_eq!(lines_of(&read), [3]);
    let Data::Stats(stats) = read.data else {
        panic!()
    };
    assert_eq!(stats.items[0].value, text("94%"));
    assert_eq!(stats.items[0].delta, Some(text("6 pts")));
    assert_eq!(stats.items[0].trend, Some(Trend::Up));
    assert_eq!(stats.items[1].delta, None);
}

#[test]
fn facts_keep_inline_content() {
    let read =
        read("::: facts\n- Owner: @priya\n- Due: 2026-11-24\n- nothing\n:::\n");
    assert_eq!(lines_of(&read), [3]);
    let Data::Facts(facts) = read.data else {
        panic!()
    };
    assert_eq!(facts.items[0].key, text("Owner"));
    assert_eq!(
        facts.items[0].value,
        [Inline::Mention {
            handle: "priya".into()
        }]
    );
}

#[test]
fn progress_bars() {
    let read = read(
        "::: progress\n- Replay engine · 70%\n- Overdone · 140%\n- 50%\n:::\n",
    );
    assert_eq!(lines_of(&read), [3]);
    let Data::Progress(progress) = read.data else {
        panic!()
    };
    let percents: Vec<f64> = progress.items.iter().map(|i| i.percent).collect();
    assert_eq!(percents, [70.0, 100.0]);
}

#[test]
fn cards_and_steps() {
    let read = self::read(
        "::: cards\n- 🚀 **Launch** Ship it\n- **Plain** card\n:::\n",
    );
    let Data::Cards(cards) = read.data else {
        panic!()
    };
    assert_eq!(cards.cards[0].icon.as_deref(), Some("🚀"));
    assert_eq!(cards.cards[0].title, text("Launch"));
    assert_eq!(cards.cards[0].body, text("Ship it"));
    assert_eq!(cards.cards[1].icon, None);

    let read =
        self::read("::: steps\n1. **Install** the CLI\n2. Sign in\n:::\n");
    let Data::Steps(steps) = read.data else {
        panic!()
    };
    assert_eq!(steps.steps[0].body, text("the CLI"));
    assert_eq!(steps.steps[1].title, text("Sign in"));
}

#[test]
fn tabs_split_at_level_two_headings() {
    let read = read(
        "::: tabs\nIntro\n\n## macOS\n\nUse brew.\n\n## Linux\n\nUse apt.\n\
            :::\n",
    );
    assert_eq!(lines_of(&read), [0]);
    let Data::Tabs(tabs) = read.data else {
        panic!()
    };
    assert_eq!(tabs.tabs.len(), 3);
    assert!(tabs.tabs[0].title.is_empty());
    assert_eq!(tabs.tabs[1].title, text("macOS"));
    assert_eq!(tabs.tabs[2].blocks.len(), 1);
}

#[test]
fn data_serializes_with_a_type_tag() {
    let read = read("```timeline\nQ1 | Plan\n```\n");
    let json = serde_json::to_value(&read.data).unwrap();
    assert_eq!(json["type"], "timeline");
    assert_eq!(json["items"][0]["title"], "Plan");
}

#[test]
fn corpus_data_fences_read_cleanly() {
    let src = include_str!("../../../../corpus/format/fences/data-fences.md");
    let parsed = crate::parse(src);
    let reads: Vec<Read<Data>> = parsed
        .document
        .blocks
        .iter()
        .filter_map(Data::from_block)
        .collect();
    assert_eq!(reads.len(), 6);
    for read in reads {
        assert!(read.diagnostics.is_empty(), "{read:?}");
    }
}

#[test]
fn calculator_parentheses_in_formulas_are_not_units() {
    let read = read(
        "```calculator\na: 1\nb: 2\nc: 3\nx: X = a * (b + c)\ny: Y = round(a)\n\
         z: Z = (a + b) * c (%)\n```\n",
    );
    let Data::Calculator(calc) = read.data else {
        panic!()
    };
    let outputs: Vec<(&str, Option<&str>)> = calc
        .outputs
        .iter()
        .map(|o| (o.formula.as_str(), o.unit.as_deref()))
        .collect();
    assert_eq!(
        outputs,
        [
            ("a * (b + c)", None),
            ("round(a)", None),
            ("(a + b) * c", Some("%")),
        ]
    );
}
