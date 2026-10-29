// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { keyValues, KROKI, parseChart, parseCsv } from "./fence-preview";

describe("keyValues", () => {
  it("splits on the first colon and skips lines without a key", () => {
    expect(keyValues("title: A: B\nno colon\n: empty key\n  type : bar ")).toEqual([
      ["title", "A: B"],
      ["type", "bar"],
    ]);
  });
});

describe("parseChart", () => {
  it("reads labels and values options", () => {
    expect(
      parseChart("title: Weekly\ntype: line\nlabels: [W1, W2]\nvalues: [1, 2.5]\nunit: ms"),
    ).toEqual({ title: "Weekly", type: "line", labels: ["W1", "W2"], values: [1, 2.5] });
  });

  it("turns pie-style lines into the series, defaulting to a bar chart", () => {
    expect(parseChart("Search: 40\nBrowse: -3.5\nNotes: many")).toEqual({
      title: "",
      type: "bar",
      labels: ["Search", "Browse"],
      values: [40, -3.5],
    });
  });

  it("returns undefined without values or with a value that is not a number", () => {
    expect(parseChart("title: Empty")).toBeUndefined();
    expect(parseChart("values: [1, two]")).toBeUndefined();
    expect(parseChart("values: []")).toBeUndefined();
  });

  it("accepts lists without brackets", () => {
    expect(parseChart("labels: a, b\nvalues: 3, 4")?.labels).toEqual(["a", "b"]);
  });
});

describe("parseCsv", () => {
  it("splits rows and cells, trimming and skipping blank lines", () => {
    expect(parseCsv("Name, Value\n\n Alpha , 1 \nBeta,2\n")).toEqual([
      ["Name", "Value"],
      ["Alpha", "1"],
      ["Beta", "2"],
    ]);
    expect(parseCsv("")).toEqual([]);
  });
});

describe("KROKI", () => {
  it("lists the sidecar diagram languages", () => {
    expect(KROKI.has("plantuml")).toBe(true);
    expect(KROKI.has("mermaid")).toBe(false);
  });
});
