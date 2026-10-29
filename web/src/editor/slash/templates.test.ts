// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { parseChart } from "../nodes/fence-preview";
import { NODES } from "../schema/blocks.gen";

import { SLASH_ITEMS } from "./items";
import { TEMPLATES } from "./templates";

const NODE_NAMES = new Set<string>(NODES.map((node) => node.name));

describe("TEMPLATES", () => {
  it("has a template for every slash entry that names one", () => {
    const named = SLASH_ITEMS.flatMap((entry) => entry.template ?? []);
    expect(named.length).toBeGreaterThan(0);
    expect(named.filter((name) => !(name in TEMPLATES))).toEqual([]);
  });

  it.each(Object.entries(TEMPLATES))("%s builds a node the schema defines", (_name, build) => {
    const content = build();
    expect(NODE_NAMES.has(content.type ?? "")).toBe(true);
    expect(build()).toEqual(content);
  });

  it("starts the chart from an example that previews", () => {
    const chart = TEMPLATES["chart"]?.();
    expect(parseChart(chart?.content?.[0]?.text ?? "")?.values).toEqual([120, 185, 160, 240]);
  });

  it("makes the requested number of columns", () => {
    expect(TEMPLATES["columns3"]?.().content).toHaveLength(3);
  });
});
