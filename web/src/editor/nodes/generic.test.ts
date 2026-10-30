// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it } from "vitest";

import { nodeSpec } from "../schema/spec";

import { Column, genericNode } from "./generic";

describe("genericNode", () => {
  it("copies content and group from the schema", () => {
    const callout = genericNode("callout", { role: "note" });
    const spec = nodeSpec("callout");
    expect(callout.config.content).toBe(spec.content);
    expect(callout.config.group).toBe(spec.group);
  });

  it("leaves out an empty group or content", () => {
    expect(Column.config.group).toBeUndefined();
    expect(genericNode("toc").config.content).toBeUndefined();
  });

  it.each([
    "callout",
    "columns",
    "column",
    "tabs",
    "heading",
    "paragraph",
  ] as const)("keeps the isolating and defining flags of %s", (name) => {
    const spec = nodeSpec(name);
    const node = genericNode(name);
    expect(node.config.isolating === true).toBe(spec.isolating === true);
    expect(node.config.defining === true).toBe(spec.defining === true);
  });
});
