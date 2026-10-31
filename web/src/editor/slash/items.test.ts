// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { SECTIONS } from "../schema/blocks.gen";

import { filterSlashItems, SLASH_ITEMS, type SlashItem } from "./items";

const titles = (items: readonly SlashItem[]) => items.map((item) => item.title);

function item(title: string, overrides: Partial<SlashItem> = {}): SlashItem {
  return {
    id: title,
    node: "paragraph",
    title,
    description: "",
    keywords: [],
    section: "Basic",
    icon: "notes",
    attrs: {},
    ...overrides,
  };
}

describe("SLASH_ITEMS", () => {
  it("has unique ids and is grouped in section order", () => {
    expect(new Set(SLASH_ITEMS.map((entry) => entry.id)).size).toBe(
      SLASH_ITEMS.length,
    );
    const sections: readonly string[] = SECTIONS;
    const order = SLASH_ITEMS.map((entry) => sections.indexOf(entry.section));
    expect(order).toEqual(order.toSorted((a, b) => a - b));
  });

  it("names the node each entry inserts", () => {
    expect(
      SLASH_ITEMS.find((entry) => entry.title === "Heading 2"),
    ).toMatchObject({
      node: "heading",
      attrs: { level: 2 },
    });
  });
});

describe("filterSlashItems", () => {
  it("returns every entry for an empty or punctuation-only query", () => {
    expect(filterSlashItems("")).toEqual(SLASH_ITEMS);
    expect(filterSlashItems(" -- ")).toHaveLength(SLASH_ITEMS.length);
  });

  it("ranks exact, prefix, word, keyword then substring matches", () => {
    const items = [
      item("Substring", { description: "about tables" }),
      item("Keyword", { keywords: ["tablet"] }),
      item("Data table"),
      item("Tables"),
      item("Table"),
      item("Nothing"),
    ];
    expect(titles(filterSlashItems("Table", items))).toEqual([
      "Table",
      "Tables",
      "Data table",
      "Keyword",
      "Substring",
    ]);
  });

  it("matches inside a title", () => {
    expect(
      titles(filterSlashItems("ble", [item("Table"), item("Text")])),
    ).toEqual(["Table"]);
  });

  it("keeps schema order between equal ranks", () => {
    expect(titles(filterSlashItems("callout")).slice(0, 2)).toEqual([
      "Info callout",
      "Warning callout",
    ]);
  });

  it("finds real entries by keyword", () => {
    expect(titles(filterSlashItems("kanban"))).toContain("Board");
    expect(filterSlashItems("zzzz-no-match")).toEqual([]);
  });
});
