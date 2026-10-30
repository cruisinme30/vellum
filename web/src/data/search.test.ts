// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { filterHits, highlight, matchesFilters, modifiedFrom, searchTitles } from "./search";
import type { PageMeta } from "./types";

function page(id: string, title: string, extra: Partial<PageMeta> = {}): PageMeta {
  return {
    id,
    spaceId: "s1",
    parentId: undefined,
    position: 0,
    title,
    icon: "📄",
    status: "draft",
    owner: "priya",
    updatedAt: "2026-10-20T10:00:00Z",
    starred: false,
    ...extra,
  };
}

const NOW = Date.parse("2026-10-29T12:00:00Z");

describe("highlight", () => {
  it("marks every query word, case-insensitively, and keeps the text whole", () => {
    const runs = highlight("Gateway architecture: the gateway routes", "GATEWAY routes");
    expect(runs.map((run) => run.text).join("")).toBe("Gateway architecture: the gateway routes");
    expect(runs.filter((run) => run.hit).map((run) => run.text)).toEqual([
      "Gateway",
      "gateway",
      "routes",
    ]);
  });

  it("strips quotes and punctuation from the query and merges overlapping words", () => {
    expect(highlight("plans planned", '"plan," plann')).toEqual([
      { text: "plan", hit: true },
      { text: "s ", hit: false },
      { text: "plann", hit: true },
      { text: "ed", hit: false },
    ]);
    expect(highlight("text", "  ")).toEqual([{ text: "text", hit: false }]);
    expect(highlight("", "x")).toEqual([]);
  });
});

describe("filters", () => {
  const mine = page("a", "Mine", {
    owner: "ana",
    starred: true,
    updatedAt: "2026-10-28T09:00:00Z",
  });
  const old = page("b", "Old", { updatedAt: "2025-01-01T00:00:00Z", spaceId: "s2" });

  it("checks owner, starred, space and age", () => {
    expect(matchesFilters(mine, undefined, "ana", NOW)).toBe(true);
    expect(matchesFilters(old, { owner: "me" }, "ana", NOW)).toBe(false);
    expect(matchesFilters(mine, { owner: "me", starred: true }, "ana", NOW)).toBe(true);
    expect(matchesFilters(old, { starred: true }, "ana", NOW)).toBe(false);
    expect(matchesFilters(old, { spaceId: "s2" }, "ana", NOW)).toBe(true);
    expect(matchesFilters(mine, { spaceId: "s2" }, "ana", NOW)).toBe(false);
    expect(matchesFilters(mine, { modifiedDays: 7 }, "ana", NOW)).toBe(true);
    expect(matchesFilters(old, { modifiedDays: 365 }, "ana", NOW)).toBe(false);
    expect(matchesFilters(page("c", "x", { updatedAt: "" }), { modifiedDays: 7 }, "ana", NOW)).toBe(
      false,
    );
  });

  it("filters hits and turns an age into a server time", () => {
    const hits = [mine, old].map((each) => ({ page: each, snippet: "" }));
    expect(filterHits(hits, { owner: "me" }, "ana", NOW).map((hit) => hit.page.id)).toEqual(["a"]);
    expect(modifiedFrom({ modifiedDays: 1 }, NOW)).toBe(NOW - 86_400_000);
    expect(modifiedFrom({}, NOW)).toBeUndefined();
  });
});

describe("searchTitles", () => {
  const pages = [
    page("1", "Roadmap 2027"),
    page("2", "The road ahead", { starred: true }),
    page("3", "Q4 roadmap review", { updatedAt: "2026-10-28T00:00:00Z" }),
    page("4", "Railroads"),
    page("5", "Roadmap folder", { folder: true }),
    page("6", "Review of the 2027 plan"),
  ];

  it("ranks prefixes, then word starts, then inside words, and skips folders", () => {
    expect(searchTitles(pages, " road ").map((each) => each.id)).toEqual(["1", "2", "3", "4"]);
    expect(searchTitles(pages, "2027 review").map((each) => each.id)).toEqual(["6"]);
    expect(searchTitles(pages, "")).toEqual([]);
    expect(searchTitles(pages, "road", 2)).toHaveLength(2);
  });

  it("answers within the 20 ms budget for a large workspace", () => {
    const many = Array.from({ length: 20_000 }, (_, index) =>
      page(String(index), `Page ${String(index)} about launch planning and roadmaps`),
    );
    let best = Number.POSITIVE_INFINITY;
    for (let run = 0; run < 5; run += 1) {
      const started = performance.now();
      searchTitles(many, "roadmap");
      best = Math.min(best, performance.now() - started);
    }
    expect(best).toBeLessThan(20);
  });
});
