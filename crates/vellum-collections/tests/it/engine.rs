// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! End-to-end tests on a small Northstar workspace: requirements linked
//! two-way to releases, with rollups, formulas, views, CSV and embeds.

use std::collections::{BTreeMap, BTreeSet};

use vellum_collections::csv;
use vellum_collections::{
    ChartSource, Collection, CollectionEmbed, Computer, DateValue, Edit,
    EditError, RowRef, SchemaError, Value, ViewKind, Workspace, view,
};
use vellum_kernel::{CollectionId, DocId};

const REQS: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C30";
const RELS: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C31";
const R1: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C41";
const R2: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C42";
const R3: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C43";
const R4: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C44";
const V10: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C51";
const V11: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C52";

#[cfg(test)]
fn reqs_schema() -> String {
    format!(
        r"id: {REQS}
title: Requirements tracker
properties:
  - {{key: t, name: Requirement, type: title}}
  - {{key: pr, name: Priority, type: select, options: [P0, P1, P2]}}
  - {{key: own, name: Owner, type: person}}
  - {{key: e, name: Target, type: date}}
  - {{key: est, name: Estimate, type: number}}
  - {{key: dn, name: Done, type: checkbox}}
  - {{key: tags, name: Tags, type: multi, options: [api, ux]}}
  - {{key: rel, name: Release, type: relation, to: {RELS}, back: rq}}
  - {{key: f1, name: Remaining, type: formula, f: 'if(Done, 0, Estimate)'}}
  - {{key: rn, name: Release points, type: rollup, via: rel, prop: pts, agg: sum}}
  - {{key: c, name: Created, type: created}}
views:
  - {{name: All, type: table, sort: est desc}}
  - {{name: By priority, type: board, group: Priority, hide: [Owner, Tags]}}
  - {{name: Open, type: list, filter: Done is false}}
  - {{name: Schedule, type: calendar, date: Target}}
"
    )
}

#[cfg(test)]
fn rels_schema() -> String {
    format!(
        r"id: {RELS}
title: Releases
properties:
  - {{key: t, name: Release, type: title}}
  - {{key: rq, name: Requirements, type: relation, to: {REQS}, back: rel}}
  - {{key: pts, name: Total points, type: rollup, via: rq, prop: est, agg: sum}}
  - {{key: left, name: Points left, type: rollup, via: rq, prop: f1, agg: sum}}
  - {{key: done, name: Done, type: rollup, via: rq, prop: dn, agg: pct}}
  - {{key: n, name: Count, type: rollup, via: rq, prop: t, agg: count}}
"
    )
}

#[cfg(test)]
fn row(id: &str, fields: &str) -> String {
    format!("---\nid: {id}\n{fields}---\nNotes.\n")
}

#[cfg(test)]
fn northstar() -> Workspace {
    let requirement_rows = [
        (
            "rows/r1.md",
            row(
                R1,
                &format!(
                    "t: R1 · Replay 7 days\npr: P0\nown: dev\ne: 2026-10-28\n\
                        est: 8\ndn: true\ntags: [api]\nrel: [{V10}]\n"
                ),
            ),
        ),
        (
            "rows/r2.md",
            row(
                R2,
                &format!(
                    "t: R2 · Diff report\npr: P0\nown: priya\ne: 2026-11-05\n\
                        est: 5\ntags: [api, ux]\nrel: [{V10}]\n"
                ),
            ),
        ),
        (
            "rows/r3.md",
            row(
                R3,
                &format!(
                    "t: R3 · PDF report\npr: P1\nown: maya\ne: 2026-11-18\n\
                        est: 3\nrel: [{V10}]\n"
                ),
            ),
        ),
        (
            "rows/r4.md",
            row(
                R4,
                &format!(
                    "t: R4 · Nightly simulations\npr: P2\nest: 8\n\
                        rel: [{V11}]\n"
                ),
            ),
        ),
    ];
    let release_rows = [
        (
            "rows/1-0.md",
            row(
                V10,
                &format!("t: 1.0 Policy simulator\nrq: [{R1}, {R2}, {R3}]\n"),
            ),
        ),
        (
            "rows/1-1.md",
            row(V11, &format!("t: 1.1 Nightly simulations\nrq: [{R4}]\n")),
        ),
    ];
    let (requirements, d1) =
        Collection::load(&reqs_schema(), requirement_rows).unwrap();
    let (releases, d2) =
        Collection::load(&rels_schema(), release_rows).unwrap();
    assert!(d1.is_empty() && d2.is_empty(), "{d1:?} {d2:?}");
    let mut ws = Workspace::new();
    ws.insert(requirements).unwrap();
    ws.insert(releases).unwrap();
    ws
}

#[cfg(test)]
fn cid(s: &str) -> CollectionId {
    s.parse().unwrap()
}

#[cfg(test)]
fn did(s: &str) -> DocId {
    s.parse().unwrap()
}

#[cfg(test)]
fn now() -> DateValue {
    DateValue::parse("2026-10-29T08:00").unwrap()
}

#[cfg(test)]
fn cell(ws: &Workspace, c: &str, r: &str, key: &str) -> String {
    let mut computer = Computer::new(ws, now());
    match computer.value(cid(c), did(r), key) {
        Ok(v) => v.to_string(),
        Err(e) => format!("error: {}", e.message),
    }
}

#[test]
fn rollups_and_formulas_compute() {
    let ws = northstar();
    assert_eq!(cell(&ws, RELS, V10, "pts"), "16");
    assert_eq!(cell(&ws, RELS, V10, "left"), "8");
    assert_eq!(cell(&ws, RELS, V10, "n"), "3");
    assert_eq!(cell(&ws, RELS, V10, "done"), "0.3333333333");
    assert_eq!(cell(&ws, REQS, R1, "f1"), "0");
    assert_eq!(cell(&ws, REQS, R2, "f1"), "5");
    // A rollup of a rollup, across the relation and back.
    assert_eq!(cell(&ws, REQS, R4, "rn"), "8");
    assert!(cell(&ws, REQS, R1, "c").starts_with("2024-"));
}

#[test]
fn relations_stay_two_way() {
    let mut ws = northstar();
    let changes = ws
        .apply(Edit::UpdateRow {
            collection: cid(REQS),
            row: did(R4),
            values: BTreeMap::from([("Release".into(), Value::text(V10))]),
            body: None,
        })
        .unwrap();
    let written: BTreeSet<RowRef> = [(REQS, R4), (RELS, V10), (RELS, V11)]
        .into_iter()
        .map(|(c, r)| RowRef {
            collection: cid(c),
            row: did(r),
        })
        .collect();
    assert_eq!(changes.written, written);
    let rels = ws.collection(cid(RELS)).unwrap();
    assert_eq!(rels.row(did(V10)).unwrap().links("rq").len(), 4);
    assert!(rels.row(did(V11)).unwrap().links("rq").is_empty());
    assert_eq!(cell(&ws, RELS, V10, "pts"), "24");
    // Every requirement's release rollup depends on the release totals.
    let affected = ws.affected(&changes);
    assert!(affected.contains(&RowRef {
        collection: cid(REQS),
        row: did(R1)
    }));

    // Deleting a row removes every link to it.
    let changes = ws
        .apply(Edit::DeleteRow {
            collection: cid(REQS),
            row: did(R1),
        })
        .unwrap();
    assert!(changes.deleted.contains(&RowRef {
        collection: cid(REQS),
        row: did(R1)
    }));
    assert_eq!(cell(&ws, RELS, V10, "pts"), "16");
    assert!(ws.affected(&changes).contains(&RowRef {
        collection: cid(RELS),
        row: did(V10)
    }));
}

#[test]
fn creating_a_row_links_the_other_side() {
    let mut ws = northstar();
    let id = DocId::generate();
    let changes = ws
        .apply(Edit::CreateRow {
            collection: cid(REQS),
            id: Some(id),
            values: BTreeMap::from([
                ("Requirement".into(), Value::text("R5 · Alerts")),
                ("est".into(), Value::Number(2.0)),
                ("Release".into(), Value::List(vec![Value::text(V11)])),
            ]),
            body: "Alert on regressions.\n".into(),
        })
        .unwrap();
    assert_eq!(changes.written.len(), 2);
    assert_eq!(cell(&ws, RELS, V11, "pts"), "10");
}

#[test]
fn edits_are_validated() {
    let mut ws = northstar();
    let set = |key: &str, v: Value| Edit::UpdateRow {
        collection: cid(REQS),
        row: did(R2),
        values: BTreeMap::from([(key.to_owned(), v)]),
        body: None,
    };
    assert!(matches!(
        ws.apply(set("Estimate", Value::text("lots"))),
        Err(EditError::WrongType { .. })
    ));
    assert!(matches!(
        ws.apply(set("Remaining", Value::Number(1.0))),
        Err(EditError::Computed(_))
    ));
    assert!(matches!(
        ws.apply(set("Nope", Value::Number(1.0))),
        Err(EditError::UnknownProperty(_))
    ));
    assert!(matches!(
        ws.apply(set("Release", Value::text(R1))),
        Err(EditError::UnknownRow(_))
    ));
    assert!(matches!(
        ws.apply(Edit::DeleteRow {
            collection: cid(REQS),
            row: did(V10)
        }),
        Err(EditError::UnknownRow(_))
    ));
    // Nothing changed.
    assert_eq!(cell(&ws, REQS, R2, "est"), "5");
}

#[test]
fn views_filter_sort_and_group() {
    let ws = northstar();
    let schema = ws.collection(cid(REQS)).unwrap().schema().clone();
    let mut computer = Computer::new(&ws, now());

    let all = view::query(
        &mut computer,
        cid(REQS),
        schema.view(Some("All")).unwrap(),
    )
    .unwrap();
    let titles: Vec<&str> = all.groups[0]
        .rows
        .iter()
        .map(|r| r.title.as_str())
        .collect();
    assert_eq!(
        titles,
        [
            "R1 · Replay 7 days",
            "R4 · Nightly simulations",
            "R2 · Diff report",
            "R3 · PDF report"
        ]
    );
    let rel_col = all.columns.iter().position(|c| c.key == "rel").unwrap();
    assert_eq!(
        all.groups[0].rows[0].cells[rel_col].display,
        "1.0 Policy simulator"
    );

    let board = view::query(
        &mut computer,
        cid(REQS),
        schema.view(Some("by priority")).unwrap(),
    )
    .unwrap();
    assert_eq!(board.kind, ViewKind::Board);
    let groups: Vec<(&str, usize)> = board
        .groups
        .iter()
        .map(|g| (g.label.as_str(), g.rows.len()))
        .collect();
    assert_eq!(groups, [("P0", 2), ("P1", 1), ("P2", 1)]);
    assert!(
        board
            .columns
            .iter()
            .all(|c| c.key != "own" && c.key != "tags")
    );

    let open = view::query(
        &mut computer,
        cid(REQS),
        schema.view(Some("Open")).unwrap(),
    )
    .unwrap();
    assert_eq!(open.total, 3);

    let cal = view::query(
        &mut computer,
        cid(REQS),
        schema.view(Some("Schedule")).unwrap(),
    )
    .unwrap();
    let dated = cal.groups[0]
        .rows
        .iter()
        .filter(|r| r.span.is_some())
        .count();
    assert_eq!(dated, 3);

    let mut by_tag = schema.view(None).unwrap().clone();
    by_tag.group = Some("tags".into());
    let tagged = view::query(&mut computer, cid(REQS), &by_tag).unwrap();
    let groups: Vec<(&str, usize)> = tagged
        .groups
        .iter()
        .map(|g| (g.label.as_str(), g.rows.len()))
        .collect();
    assert_eq!(groups, [("api", 2), ("ux", 1), ("No Tags", 2)]);

    let json = serde_json::to_value(&all).unwrap();
    assert_eq!(json["kind"], "table");
    assert_eq!(json["columns"][0]["type"], "title");
}

#[test]
fn filters_cover_every_operator() {
    let ws = northstar();
    let schema = ws.collection(cid(REQS)).unwrap().schema().clone();
    let mut computer = Computer::new(&ws, now());
    let count = |computer: &mut Computer<'_>, filter: &str| {
        let mut v = schema.view(None).unwrap().clone();
        v.filters =
            vec![vellum_collections::Filter::parse(filter, &schema).unwrap()];
        view::query(computer, cid(REQS), &v).unwrap().total
    };
    assert_eq!(count(&mut computer, "Priority is P0"), 2);
    assert_eq!(count(&mut computer, "Priority != P0"), 2);
    assert_eq!(count(&mut computer, "Requirement contains report"), 2);
    assert_eq!(count(&mut computer, "Requirement not contains report"), 2);
    assert_eq!(count(&mut computer, "Owner is empty"), 1);
    assert_eq!(count(&mut computer, "Owner is not empty"), 3);
    assert_eq!(count(&mut computer, "Estimate > 5"), 2);
    assert_eq!(count(&mut computer, "Estimate <= 5"), 2);
    assert_eq!(count(&mut computer, "Target < @today"), 1);
    assert_eq!(count(&mut computer, "Target >= 2026-11-05"), 2);
    assert_eq!(count(&mut computer, "Tags is ux"), 1);
    assert_eq!(
        count(&mut computer, "Release is \"1.0 Policy simulator\""),
        3
    );
    assert_eq!(count(&mut computer, "Done is true"), 1);
    assert_eq!(count(&mut computer, "Remaining >= 5"), 2);
}

#[test]
fn formula_errors_show_inline() {
    let mut ws = northstar();
    let yaml = reqs_schema().replace(
        "views:",
        "  - {key: bad, name: Ratio, type: formula, f: 'Estimate / (Estimate - \
            Estimate)'}\nviews:",
    );
    let schema = vellum_collections::Schema::from_yaml(&yaml).unwrap();
    ws.set_schema(schema.clone()).unwrap();
    let mut computer = Computer::new(&ws, now());
    let result =
        view::query(&mut computer, cid(REQS), schema.view(None).unwrap())
            .unwrap();
    let col = result.columns.iter().position(|c| c.key == "bad").unwrap();
    let c = &result.groups[0].rows[0].cells[col];
    assert_eq!(c.display, "⚠ division by zero");
    let span = c.error.as_ref().unwrap().span.unwrap();
    assert_eq!((span.start, span.end), (0, 32));
}

#[test]
fn cross_collection_cycles_are_reported() {
    let reqs = reqs_schema().replace(
        "views:",
        "  - {key: x, name: X, type: rollup, via: rel, prop: y, agg: sum}\n\
            views:",
    );
    let releases = rels_schema()
        + "  - {key: y, name: Y, type: rollup, via: rq, prop: x, agg: sum}\n";
    let (a, _) = Collection::load(
        &reqs,
        [("rows/r1.md", row(R1, &format!("t: R1\nrel: [{V10}]\n")))],
    )
    .unwrap();
    let (b, _) = Collection::load(
        &releases,
        [("rows/v.md", row(V10, &format!("t: 1.0\nrq: [{R1}]\n")))],
    )
    .unwrap();
    let mut ws = Workspace::new();
    ws.insert(a).unwrap();
    ws.insert(b).unwrap();
    assert!(cell(&ws, REQS, R1, "x").starts_with("error:"));
}

#[test]
fn mismatched_relation_pairs_are_rejected() {
    let mut ws = northstar();
    let bad = rels_schema().replace("back: rel", "back: t");
    let (c, _) =
        Collection::load(&bad, std::iter::empty::<(&str, &str)>()).unwrap();
    assert!(matches!(
        ws.insert(c),
        Err(SchemaError::RelationBack { .. })
    ));
    assert_eq!(ws.collection(cid(RELS)).unwrap().len(), 2);
}

#[test]
fn files_round_trip() {
    let ws = northstar();
    let c = ws.collection(cid(REQS)).unwrap();
    let files = c.files().unwrap();
    assert_eq!(files[0].0, "schema.yaml");
    assert!(files.iter().any(|(p, _)| p == "rows/r1-replay-7-days.md"));
    let (again, diags) =
        Collection::load(&files[0].1, files[1..].iter().cloned()).unwrap();
    assert!(diags.is_empty(), "{diags:?}\n{}", files[0].1);
    assert_eq!(again.files().unwrap(), files);
}

#[test]
fn csv_round_trips() {
    let ws = northstar();
    let schema = ws.collection(cid(REQS)).unwrap().schema().clone();
    let mut computer = Computer::new(&ws, now());
    let text = csv::export(
        &mut computer,
        cid(REQS),
        schema.view(Some("All")).unwrap(),
    )
    .unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next().unwrap(),
        "Requirement,Priority,Owner,Target,Estimate,Done,Tags,Release,Remaining\
            ,Release points,Created"
    );
    assert!(text.contains(
        "R2 · Diff report,P0,priya,2026-11-05,5,No,\"api, ux\",1.0 Policy \
            simulator,5,16,"
    ));

    // Import into an empty copy of the collection.
    let mut ws2 = northstar();
    for r in [R1, R2, R3, R4] {
        let _ = ws2
            .apply(Edit::DeleteRow {
                collection: cid(REQS),
                row: did(r),
            })
            .unwrap();
    }
    let import = csv::import_into(&text, &ws2, cid(REQS)).unwrap();
    assert_eq!(import.rows.len(), 4);
    assert_eq!(
        import.skipped_columns,
        ["Remaining", "Release points", "Created"]
    );
    assert!(import.diagnostics.is_empty(), "{:?}", import.diagnostics);
    for edit in import.edits() {
        let _ = ws2.apply(edit).unwrap();
    }
    assert_eq!(cell(&ws2, RELS, V10, "pts"), "16");

    // A brand-new collection infers its types.
    let new = csv::import_new(
        "Name,Points,Shipped,Due,Link,Team\n\
            A,1,yes,2026-01-01,https://northstar.dev/a,eng\n\
            B,2.5,no,2026-02-01,https://northstar.dev/b,eng\nC,,yes,,,design\n\
            =cmd,3,no,2026-03-01,https://northstar.dev/c,eng\n",
        CollectionId::generate(),
        "Imported",
    )
    .unwrap();
    let kinds: Vec<&str> = new
        .schema
        .properties
        .iter()
        .map(|p| p.kind.name())
        .collect();
    assert_eq!(
        kinds,
        ["title", "number", "checkbox", "date", "url", "select"]
    );
    let c = new.into_collection();
    assert_eq!(c.len(), 4);
    let mut ws3 = Workspace::new();
    let id = c.id();
    ws3.insert(c).unwrap();
    let mut computer = Computer::new(&ws3, now());
    let all = ws3
        .collection(id)
        .unwrap()
        .schema()
        .view(None)
        .unwrap()
        .clone();
    let out = csv::export(&mut computer, id, &all).unwrap();
    assert!(
        out.contains("'=cmd"),
        "spreadsheet formulas are defused: {out}"
    );
}

/// Column names are defused like cells, and read back unchanged (#317).
#[test]
fn csv_headers_cannot_run_as_formulas() {
    let names = [
        "Name",
        "=HYPERLINK(\"https://evil.example\",\"x\")",
        "+Points",
        "@Team",
    ];
    let text = format!(
        "{}\nA,1,2,eng\n",
        names
            .map(|n| format!("\"{}\"", n.replace('"', "\"\"")))
            .join(",")
    );
    let c = csv::import_new(&text, CollectionId::generate(), "Imported")
        .unwrap()
        .into_collection();
    let id = c.id();
    let mut ws = Workspace::new();
    ws.insert(c).unwrap();
    let view = ws
        .collection(id)
        .unwrap()
        .schema()
        .view(None)
        .unwrap()
        .clone();
    let mut computer = Computer::new(&ws, now());
    let out = csv::export(&mut computer, id, &view).unwrap();
    let header = out.lines().next().unwrap();
    assert!(
        header.split(',').all(|cell| !cell
            .trim_start_matches('"')
            .starts_with(['=', '+', '-', '@'])),
        "header runs as a formula: {header}"
    );
    let again =
        csv::import_new(&out, CollectionId::generate(), "Again").unwrap();
    let read: Vec<&str> = again
        .schema
        .properties
        .iter()
        .map(|p| p.name.as_str())
        .collect();
    assert_eq!(read, names, "names survive an export and import");
}

#[test]
fn embeds_and_charts_resolve() {
    let ws = northstar();
    let mut computer = Computer::new(&ws, now());
    let embed = CollectionEmbed::parse(&format!(
        "id: {REQS}\nview: By priority\nfilter: Done is false\n\
            hide: [Release]\n"
    ))
    .unwrap();
    let result = embed.resolve(&mut computer).unwrap();
    assert_eq!(result.total, 3);
    assert!(result.columns.iter().all(|c| c.key != "rel"));

    let missing =
        CollectionEmbed::parse(&format!("id: {REQS}\nview: Nope\n")).unwrap();
    assert!(
        missing
            .resolve(&mut computer)
            .unwrap_err()
            .0
            .contains("no view named")
    );

    let attrs: BTreeMap<String, String> = [
        ("source".to_owned(), format!("collection:{REQS}")),
        ("x".to_owned(), "Priority".to_owned()),
        ("y".to_owned(), "Estimate".to_owned()),
    ]
    .into_iter()
    .collect();
    let chart = ChartSource::from_attrs(&attrs)
        .unwrap()
        .unwrap()
        .resolve(&mut computer)
        .unwrap();
    assert_eq!(chart.categories, ["P0", "P1", "P2"]);
    assert_eq!(chart.series.len(), 1);
    assert_eq!(chart.series[0].values, [13.0, 3.0, 8.0]);

    let mut by_owner = attrs;
    let _ = by_owner.insert("series".into(), "Done".into());
    let _ = by_owner.insert("agg".into(), "count".into());
    let chart = ChartSource::from_attrs(&by_owner)
        .unwrap()
        .unwrap()
        .resolve(&mut computer)
        .unwrap();
    let names: Vec<&str> =
        chart.series.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Yes", "No"]);
    assert_eq!(chart.series[1].values, [1.0, 1.0, 1.0]);
}

#[test]
fn the_save_path_recomputes_affected_rows() {
    let mut ws = northstar();
    let changes = ws
        .apply(Edit::UpdateRow {
            collection: cid(REQS),
            row: did(R3),
            values: BTreeMap::from([("Estimate".into(), Value::Number(13.0))]),
            body: Some("Rescoped.\n".into()),
        })
        .unwrap();
    let affected = ws.affected(&changes);
    let mut computer = Computer::new(&ws, now());
    let rows = computer.rows(&affected);
    let release = rows
        .iter()
        .find(|(r, _)| r.row == did(V10))
        .map(|(_, c)| c)
        .unwrap();
    assert_eq!(release.cells["pts"], Ok(Value::Number(26.0)));
    let json = serde_json::to_value(release).unwrap();
    assert_eq!(json["cells"]["pts"]["value"]["value"], 26.0);
    let schema = schemars::schema_for!(vellum_collections::ComputedRow);
    assert!(
        serde_json::to_string(&schema)
            .unwrap()
            .contains("CellValue")
    );

    let err: vellum_kernel::Error = EditError::UnknownRow(did(R1)).into();
    assert_eq!(err.kind(), vellum_kernel::ErrorKind::NotFound);
    let err: vellum_kernel::Error = SchemaError::MissingId.into();
    assert_eq!(err.kind(), vellum_kernel::ErrorKind::InvalidInput);
}

// A hostile CSV must not turn a few megabytes into millions of warnings or
// minutes of work while the collections lock is held (#367).

#[test]
fn csv_import_caps_its_diagnostics() {
    let ws = northstar();
    // Two bytes per unknown release title, and one bad number per row.
    let cell = "a,".repeat(200_000);
    let rows = "r,,lots\n".repeat(5_000);
    let text =
        format!("Requirement,Release,Estimate\nx,\"{cell}\",lots\n{rows}");
    let import = csv::import_into(&text, &ws, cid(REQS)).unwrap();
    assert!(
        import.diagnostics.len() <= csv::MAX_DIAGNOSTICS + 1,
        "{} diagnostics",
        import.diagnostics.len()
    );
    let last = import.diagnostics.last().unwrap();
    assert!(last.message.contains("more problems"), "{last:?}");
}

#[test]
fn csv_import_matches_relation_titles_in_linear_time() {
    let mut ws = northstar();
    for i in 0..2_000 {
        let mut values = BTreeMap::new();
        let _ =
            values.insert("t".to_owned(), Value::Text(format!("Release {i}")));
        let _ = ws
            .apply(Edit::CreateRow {
                collection: cid(RELS),
                id: None,
                values,
                body: String::new(),
            })
            .unwrap();
    }
    // 50,000 titles against 2,000 rows: about 100 million comparisons when
    // each title scans the collection, and a few milliseconds with an index.
    let titles: Vec<String> = (0..50_000)
        .map(|i| format!("release {}", i % 2_500))
        .collect();
    let text = format!("Requirement,Release\nx,\"{}\"\n", titles.join(","));
    let started = std::time::Instant::now();
    let import = csv::import_into(&text, &ws, cid(REQS)).unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "took {elapsed:?}"
    );
    let Some(Value::List(ids)) = import.rows[0].get("rel") else {
        panic!("no relation: {:?}", import.rows[0]);
    };
    // Matches are case-insensitive and each row appears once.
    assert_eq!(ids.len(), 2_000);
}
