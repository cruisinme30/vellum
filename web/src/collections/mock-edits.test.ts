// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { createMockCollectionsStore } from "./mock";
import { COLLECTION } from "./mock-data";
import { csvLine, embedQuery, parseCsv } from "./mock-edits";
import type { ViewResult } from "./types";

const ISSUES = COLLECTION.issues;
const RELEASES = COLLECTION.releases;
const rowsOf = (result: ViewResult) => result.groups.flatMap((g) => g.rows);

describe("CSV text", () => {
  it("parses quotes, commas, doubled quotes and line breaks", () => {
    expect(parseCsv('Name,Note\r\n"Ship, then rest","He said ""go""\nnow"\n\n,\n')).toEqual([
      ["Name", "Note"],
      ["Ship, then rest", 'He said "go"\nnow'],
    ]);
  });

  it("quotes only the cells that need it, and parses back", () => {
    const cells = ["plain", "a,b", 'say "hi"', "two\nlines"];
    const line = csvLine(cells);
    expect(line).toBe('plain,"a,b","say ""hi""","two\nlines"');
    expect(parseCsv(line)).toEqual([cells]);
  });
});

describe("collection blocks", () => {
  it("read id, view, layout, repeated filters and a limit", () => {
    expect(
      embedQuery(
        "id: C1\nview: Open\ntype: board\nfilter: Status is Done\nfilter: Estimate > 1\nlimit: 5\nnoise",
      ),
    ).toEqual({
      collection_id: "C1",
      view: "Open",
      kind: "board",
      filter: ["Status is Done", "Estimate > 1"],
      limit: 5,
    });
    expect(() => embedQuery("view: Open")).toThrow(/id/);
  });
});

describe("the mock store's schema, CSV and embed operations", () => {
  it("saves, renames and deletes views by name, storing keys", async () => {
    const store = createMockCollectionsStore();
    let info = await store.saveView({
      collection_id: ISSUES,
      view: "Mine",
      kind: "board",
      group: "Status",
      hide: ["Link"],
    });
    expect(info.views.at(-1)).toMatchObject({ name: "Mine", group: "status", hidden: ["link"] });
    info = await store.saveView({ collection_id: ISSUES, view: "mine", rename_to: "Ours" });
    expect(info.views.map((v) => v.name)).toContain("Ours");
    await expect(
      store.saveView({ collection_id: ISSUES, view: "Ours", rename_to: "board" }),
    ).rejects.toThrow(/already exists/);
    info = await store.deleteView({ collection_id: ISSUES, view: "OURS" });
    expect(info.views.map((v) => v.name)).not.toContain("Ours");
    await expect(store.deleteView({ collection_id: ISSUES, view: "Ours" })).rejects.toThrow(
      /no view/,
    );
  });

  it("adds, renames and removes properties, cleaning up views", async () => {
    const store = createMockCollectionsStore();
    let info = await store.addProperty({
      collection_id: ISSUES,
      name: "Due date!",
      type: "select",
      options: [{ name: "Soon" }],
    });
    expect(info.properties.at(-1)).toMatchObject({ key: "due_date", type: "select" });
    await expect(
      store.addProperty({ collection_id: ISSUES, name: "status", type: "text" }),
    ).rejects.toThrow(/already exists/);
    info = await store.updateProperty({ collection_id: ISSUES, key: "due_date", name: "When" });
    expect(info.properties.at(-1)?.name).toBe("When");
    info = await store.deleteProperty({ collection_id: ISSUES, key: "status" });
    expect(info.properties.some((p) => p.key === "status")).toBe(false);
    const board = info.views.find((v) => v.name === "Board");
    expect(board?.group).toBeUndefined();
    expect(info.views.find((v) => v.name === "Open")?.filters).toEqual([]);
    await expect(store.deleteProperty({ collection_id: ISSUES, key: "name" })).rejects.toThrow(
      /title/,
    );
    await expect(store.deleteProperty({ collection_id: ISSUES, key: "release" })).rejects.toThrow(
      /two-way/,
    );
  });

  it("deletes a row and unlinks it from the other side", async () => {
    const store = createMockCollectionsStore();
    const release = rowsOf(await store.query({ collection_id: RELEASES }))[0];
    const detail = await store.getRow({ collection_id: RELEASES, row_id: release?.id ?? "" });
    const issues = detail.row.cells["issues"];
    const first = issues && "value" in issues ? issues.value : undefined;
    const id =
      first?.type === "list" && first.value[0]?.type === "text" ? first.value[0].value : "";
    expect(id).not.toBe("");
    const written = await store.deleteRow({ collection_id: ISSUES, row_id: id });
    expect(written.changed.deleted).toEqual([{ collection: ISSUES, row: id }]);
    expect(rowsOf(await store.query({ collection_id: ISSUES })).some((r) => r.id === id)).toBe(
      false,
    );
    const releases = await Promise.all(
      rowsOf(await store.query({ collection_id: RELEASES })).map((r) =>
        store.getRow({ collection_id: RELEASES, row_id: r.id }),
      ),
    );
    expect(JSON.stringify(releases)).not.toContain(id);
  });

  it("exports a view, checks an import, then imports it", async () => {
    const store = createMockCollectionsStore();
    const file = await store.exportCsv({ collection_id: ISSUES, view: "Open" });
    expect(file.fileName).toBe("Issues - Open.csv");
    expect(file.csv.split("\n")[0]).toMatch(/^Issue,Status/);
    const csv = 'Issue,Estimate,Labels,Blocked,Mood\nNew one,3,"bug, docs",yes,happy\n';
    const check = await store.importCsv({ collection_id: ISSUES, csv, dry_run: true });
    expect(check).toMatchObject({ written: false, rows: 1, skippedColumns: ["Mood"] });
    const before = await store.query({ collection_id: ISSUES });
    expect(before.total).toBe(8);
    const done = await store.importCsv({ collection_id: ISSUES, csv });
    expect(done.written).toBe(true);
    const after = await store.query({ collection_id: ISSUES });
    expect(after.total).toBe(9);
  });

  it("creates a collection from CSV, guessing column types", async () => {
    const store = createMockCollectionsStore();
    const made = await store.createFromCsv({
      folder: "drives/shared/Northstar/",
      title: "Hosts",
      csv: "Host,Cores,Since,Notes\nalpha,8,2026-01-02,fast\nbeta,4,2026-03-04,\n",
    });
    expect(made.collection.path).toBe("drives/shared/Northstar/Hosts.collection");
    expect(made.collection.properties.map((p) => p.type)).toEqual([
      "title",
      "number",
      "date",
      "text",
    ]);
    const rows = await store.query({ collection_id: made.collection.id });
    expect(rows.total).toBe(2);
    await expect(
      store.createFromCsv({ folder: "x", title: "hosts", csv: "Name\n" }),
    ).rejects.toThrow(/already there/);
  });

  it("answers a collection block with its view's rows", async () => {
    const store = createMockCollectionsStore();
    const embed = await store.embed(`id: ${ISSUES}\nview: Open\nlimit: 2`);
    expect(embed.title).toBe("Issues");
    expect(embed.result.view).toBe("Open");
    expect(rowsOf(embed.result)).toHaveLength(2);
  });
});
