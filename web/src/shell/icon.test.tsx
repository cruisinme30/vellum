// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { NODES } from "../editor/schema/blocks.gen";

import { DESTINATIONS } from "./destinations";
import { hasIcon, Icon } from "./icon";

const SLASH_ICONS = NODES.flatMap((node) =>
  node.slash.map((entry) => entry.icon),
);

describe("hasIcon", () => {
  it("knows every icon the slash menu uses", () => {
    expect(SLASH_ICONS.length).toBeGreaterThan(0);
    expect(SLASH_ICONS.filter((name) => !hasIcon(name))).toEqual([]);
  });

  it("knows every navigation icon", () => {
    const icons = Object.values(DESTINATIONS).map(
      (destination) => destination.icon,
    );
    expect(icons.filter((name) => !hasIcon(name))).toEqual([]);
  });

  it("rejects names that are not bundled", () => {
    expect(hasIcon("not_an_icon")).toBe(false);
  });
});

describe("Icon", () => {
  it("draws a decorative SVG path at the requested size", () => {
    const { container } = render(<Icon name="home" size={18} />);
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("aria-hidden")).toBe("true");
    expect(svg?.getAttribute("width")).toBe("18");
    expect(svg?.querySelector("path")?.getAttribute("d")).toMatch(/^M/);
  });

  it("defaults to 24 px and draws nothing for an unknown name", () => {
    const { container } = render(<Icon name="not_an_icon" />);
    expect(container.querySelector("svg")?.getAttribute("height")).toBe("24");
    expect(container.querySelector("path")?.getAttribute("d")).toBe("");
  });
});
