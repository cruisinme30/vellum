// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { createMockCollectionsStore } from "./mock";
import { COLLECTION } from "./mock-data";
import type { ViewResult } from "./types";
import { bool, date, EMPTY, list, number, text } from "./value";

const ISSUES = COLLECTION.issues;
const RELEASES = COLLECTION.releases;

const titles = (result: ViewResult) => result.groups.flatMap((g) => g.rows.map((r) => r.title));
const cell = (result: ViewResult, title: string, key: string) => {
  const column = result.columns.findIndex((c) => c.key === key);
  const row = result.groups.flatMap((g) => g.rows).find((r) => r.title === title);
  return row?.cells[column];
};

describe("the mock collections store", () => {
  it("lists collections by title with their views", async () => {
    const { collections } = await createMockCollectionsStore().list();
    expect(collections.map((c) => [c.title, c.rows])).toEqual([
      ["Issues", 8],
      ["Releases", 3],
    ]);
    expect(collections[0]?.views).toEqual([
      "All issues",
      "Board",
      "Gallery",
      "Calendar",
      "Timeline",
      "Open",
    ]);
  });

  it("describes a collection's properties and views", async () => {
    const info = await createMockCollectionsStore().info(ISSUES);
    expect(info.properties.map((p) => p.type)).toEqual([
      "title",
      "select",
      "multi",
      "person",
      "date",
      "date",
      "number",
      "checkbox",
      "url",
      "relation",
      "formula",
    ]);
    expect(info.properties.find((p) => p.key === "left")).toMatchObject({ computed: true });
    expect(info.diagnostics).toEqual([]);
    await expect(createMockCollectionsStore().info("nope")).rejects.toThrow(
      "collection nope not found",
    );
  });

  it("runs the first view, sorted, with every column", async () => {
    const result = await createMockCollectionsStore().query({ collection_id: ISSUES });
    expect(result).toMatchObject({ view: "All issues", kind: "table", total: 8, truncated: false });
    expect(result.groups).toHaveLength(1);
    expect(result.groups[0]?.value).toBeNull();
    expect(titles(result)[0]).toBe("Trace sampling knobs");
    // Rows without a due date sort last.
    expect(titles(result).at(-1)).toBe("Flaky TLS handshake test");
    expect(cell(result, "Token bucket per tenant", "release")?.display).toBe("Gateway beta");
    expect(cell(result, "Token bucket per tenant", "blocked")?.display).toBe("No");
    expect(cell(result, "Flaky TLS handshake test", "owner")).toEqual({ display: "" });
    expect(cell(result, "Trace sampling knobs", "left")?.value).toEqual(number(0));
    expect(cell(result, "Token bucket per tenant", "left")?.value).toEqual(number(5));
    expect(result.columns.find((c) => c.key === "left")?.editable).toBe(false);
  });

  it("groups a board by status, every option first", async () => {
    const result = await createMockCollectionsStore().query({
      collection_id: ISSUES,
      view: "board",
    });
    expect(result.groups.map((g) => [g.label, g.rows.length])).toEqual([
      ["Backlog", 3],
      ["In progress", 2],
      ["In review", 1],
      ["Done", 2],
    ]);
    expect(result.groups[0]?.value).toEqual(text("Backlog"));
    expect(result.columns.map((c) => c.key)).not.toContain("link");
  });

  it("groups by lists, checkboxes and relations, with an empty group last", async () => {
    const store = createMockCollectionsStore();
    const byLabel = await store.query({ collection_id: ISSUES, group: "labels" });
    expect(byLabel.groups.map((g) => g.label)).toEqual(["bug", "feature", "docs", "performance"]);
    expect(byLabel.groups[0]?.rows).toHaveLength(2);
    const byBlocked = await store.query({ collection_id: ISSUES, group: "Blocked" });
    expect(byBlocked.groups.map((g) => [g.label, g.value])).toEqual([
      ["No", bool(false)],
      ["Yes", bool(true)],
    ]);
    const byRelease = await store.query({ collection_id: ISSUES, group: "release" });
    expect(byRelease.groups.map((g) => g.label)).toEqual([
      "Gateway beta",
      "Gateway 1.0",
      "Gateway 1.1",
      "No Release",
    ]);
    expect(byRelease.groups.at(-1)?.value).toBeNull();
  });

  it("keeps an empty group on a board with no options", async () => {
    const result = await createMockCollectionsStore().query({
      collection_id: ISSUES,
      kind: "board",
      group: "owner",
      filter: ["owner is empty"],
    });
    expect(result.groups.map((g) => g.label)).toEqual(["No Owner"]);
  });

  it("filters with every operator", async () => {
    const store = createMockCollectionsStore();
    const count = async (...filter: string[]) => {
      const result = await store.query({ collection_id: ISSUES, filter });
      return result.total;
    };
    expect(await count("status is Done")).toBe(2);
    expect(await count("Status == done")).toBe(2);
    expect(await count("status = Done")).toBe(2);
    expect(await count("status != Done")).toBe(6);
    expect(await count("status not Done")).toBe(6);
    expect(await count("name contains GATEWAY")).toBe(1);
    expect(await count("name not contains gateway")).toBe(7);
    expect(await count("owner is empty")).toBe(1);
    expect(await count("owner is not empty")).toBe(7);
    expect(await count("owner is")).toBe(1);
    expect(await count("est > 3")).toBe(3);
    expect(await count("est >= 3")).toBe(5);
    expect(await count("est < 3")).toBe(2);
    expect(await count("est <= 3")).toBe(4);
    expect(await count("est is 8")).toBe(1);
    expect(await count("due < 2026-11-06")).toBe(2);
    expect(await count("due is 2026-11-06")).toBe(1);
    expect(await count("name > t")).toBe(2);
    expect(await count("blocked is true")).toBe(1);
    expect(await count("blocked is false")).toBe(7);
    expect(await count("labels is docs")).toBe(2);
    expect(await count("release is Gateway beta")).toBe(4);
    expect(await count("blocked > 1")).toBe(0);
    await expect(count("nothing here")).rejects.toThrow("cannot read filter");
    await expect(count("ghost is 1")).rejects.toThrow("cannot read filter");
  });

  it("sorts, hides and limits", async () => {
    const store = createMockCollectionsStore();
    const result = await store.query({
      collection_id: ISSUES,
      sort: ["status desc", "Estimate"],
      hide: ["labels", "name"],
      limit: 3,
    });
    expect(titles(result)).toEqual([
      "Trace sampling knobs",
      "Per-route timeouts",
      "Gateway returns 502 on cold start",
    ]);
    expect(result).toMatchObject({ total: 8, truncated: true });
    expect(result.columns.map((c) => c.key)).toContain("name");
    expect(result.columns.map((c) => c.key)).not.toContain("labels");
    const open = await store.query({ collection_id: ISSUES, view: "Open" });
    expect(titles(open)[0]).toBe("Replay protection for webhooks");
    expect(open.total).toBe(6);
    await expect(store.query({ collection_id: ISSUES, view: "Nope" })).rejects.toThrow(
      'no view named "Nope"',
    );
  });

  it("gives calendar and timeline rows a date span", async () => {
    const store = createMockCollectionsStore();
    const timeline = await store.query({ collection_id: ISSUES, view: "Timeline" });
    const rows = timeline.groups.flatMap((g) => g.rows);
    expect(rows[0]).toMatchObject({
      title: "Trace sampling knobs",
      span: { start: "2026-10-26", end: "2026-11-03" },
    });
    expect(rows.at(-1)?.span).toBeUndefined();
    const calendar = await store.query({ collection_id: ISSUES, view: "Calendar" });
    expect(calendar.groups[0]?.rows[0]?.span).toEqual({ start: "2026-11-13", end: "2026-11-13" });
    const table = await store.query({ collection_id: ISSUES });
    expect(table.groups[0]?.rows[0]?.span).toBeUndefined();
    const releases = await store.query({ collection_id: RELEASES, kind: "calendar" });
    expect(releases.groups[0]?.rows[0]?.span).toEqual({ start: "2026-11-14", end: "2026-11-14" });
  });

  it("computes rollups over relations", async () => {
    const result = await createMockCollectionsStore().query({ collection_id: RELEASES });
    expect(titles(result)).toEqual(["Gateway beta", "Gateway 1.0", "Gateway 1.1"]);
    expect(cell(result, "Gateway beta", "points")?.display).toBe("16");
    expect(cell(result, "Gateway beta", "count")?.display).toBe("4");
    expect(cell(result, "Gateway 1.1", "points")?.display).toBe("0");
  });

  it("updates a row, notifies listeners and returns its detail", async () => {
    const store = createMockCollectionsStore();
    const listener = vi.fn<() => void>();
    const stop = store.subscribe(listener);
    const { groups } = await store.query({ collection_id: ISSUES });
    const id = groups[0]?.rows[0]?.id ?? "";
    const written = await store.updateRow({
      collection_id: ISSUES,
      row_id: id,
      values: { Status: text("Backlog"), est: number(13) },
      body: "New notes.",
    });
    expect(listener).toHaveBeenCalledOnce();
    expect(written.changed.written).toEqual([{ collection: ISSUES, row: id }]);
    expect(written.row?.row.cells["left"]).toEqual({ value: number(13) });
    expect(written.row?.body).toBe("New notes.");
    expect(written.row?.path).toBe(`engineering/issues.collection/rows/${id}.md`);
    stop();
    await store.updateRow({ collection_id: ISSUES, row_id: id, values: { est: EMPTY } });
    expect(listener).toHaveBeenCalledOnce();
    const detail = await store.getRow({ collection_id: ISSUES, row_id: id });
    expect(detail.row.cells["est"]).toEqual({ value: EMPTY });
    expect(detail.body).toBe("New notes.");
  });

  it("rejects writes the server would refuse", async () => {
    const store = createMockCollectionsStore();
    const { groups } = await store.query({ collection_id: ISSUES });
    const row = groups[0]?.rows[0]?.id ?? "";
    const update = (values: Record<string, ReturnType<typeof text>>) =>
      store.updateRow({ collection_id: ISSUES, row_id: row, values });
    await expect(update({ left: number(1) })).rejects.toThrow(
      "Points left is computed and cannot be set",
    );
    await expect(update({ est: text("many") })).rejects.toThrow(
      "Estimate expects a number value, not text",
    );
    await expect(update({ status: text("Shipped") })).rejects.toThrow(
      '"Shipped" is not an option of Status',
    );
    await expect(update({ labels: list([text("nope")]) })).rejects.toThrow("not an option");
    await expect(update({ release: list([text("missing")]) })).rejects.toThrow(
      "links to a row that does not exist: missing",
    );
    await expect(update({ ghost: text("x") })).rejects.toThrow('no property "ghost"');
    await expect(update({ due: date("2026-12-01"), owner: text("dev") })).resolves.toBeDefined();
    await expect(store.getRow({ collection_id: ISSUES, row_id: "nope" })).rejects.toThrow(
      "row nope not found",
    );
  });

  it("creates rows and mirrors two-way relations", async () => {
    const store = createMockCollectionsStore();
    const releases = await store.query({ collection_id: RELEASES });
    const next = releases.groups[0]?.rows.find((r) => r.title === "Gateway 1.1")?.id ?? "";
    const created = await store.createRow({
      collection_id: ISSUES,
      values: { name: text("New issue"), release: list([text(next)]) },
    });
    expect(created.row?.title).toBe("New issue");
    expect(created.row?.body).toBe("");
    const after = await store.query({ collection_id: RELEASES });
    expect(cell(after, "Gateway 1.1", "count")?.display).toBe("2");
    const id = created.row?.row.id ?? "";
    await store.updateRow({ collection_id: ISSUES, row_id: id, values: { release: EMPTY } });
    const removed = await store.query({ collection_id: RELEASES });
    expect(cell(removed, "Gateway 1.1", "count")?.display).toBe("1");
    const blank = await store.createRow({ collection_id: ISSUES, body: "Body" });
    expect(blank.row).toMatchObject({ title: "", body: "Body" });
    const { collections } = await store.list();
    expect(collections[0]?.rows).toBe(10);
  });

  it("formats percentages", async () => {
    const store = createMockCollectionsStore({
      collections: [
        {
          id: "c",
          title: "Scores",
          path: "scores.collection",
          properties: [
            { key: "name", name: "Name", type: "title", computed: false },
            { key: "pct", name: "Score", type: "number", computed: false, format: "percent" },
          ],
          views: [{ name: "All", kind: "table", filters: [], sorts: [], hidden: [] }],
          formulas: {},
        },
      ],
      rows: { c: [{ id: "r", values: { name: text("A"), pct: number(0.25) }, body: "" }] },
    });
    const result = await store.query({ collection_id: "c" });
    expect(cell(result, "A", "pct")?.display).toBe("25%");
    expect(await store.query({ collection_id: "c", group: "nope" })).toMatchObject({ total: 1 });
  });
});
