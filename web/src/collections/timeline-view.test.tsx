// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { renderApp } from "../test/render-app";

import { createMockCollectionsStore } from "./mock";
import { COLLECTION } from "./mock-data";

describe("TimelineView", () => {
  it("draws a bar from each row's start to its due date", async () => {
    renderApp(`/data/${COLLECTION.issues}?view=Timeline`);
    const bars = await screen.findByRole("list", { name: "Timeline" });
    const items = within(bars).getAllByRole("button");
    expect(items).toHaveLength(7);
    const first = items[0];
    // The scale starts two days before the first bar, on October 24.
    expect(first?.textContent).toBe("Trace sampling knobsfrom Oct 26 to Nov 3");
    expect(first?.style.gridColumn).toBe("3 / 12");
    const scale = document.querySelector(".vl-timeline-scale");
    expect(scale?.children[0]?.textContent).toBe("Oct24");
    expect(scale?.children[8]?.textContent).toBe("Nov1");
    expect(scale?.getAttribute("aria-hidden")).toBe("true");
  });

  it("lists rows without dates after the chart, and opens rows", async () => {
    const { user } = renderApp(`/data/${COLLECTION.issues}?view=Timeline`);
    const undated = await screen.findByRole("region", { name: "No dates" });
    await user.click(within(undated).getByRole("button", { name: "Flaky TLS handshake test" }));
    expect(await screen.findByRole("dialog", { name: "Flaky TLS handshake test" })).toBeDefined();
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: /^Per-route timeouts/ }));
    expect(await screen.findByRole("dialog", { name: "Per-route timeouts" })).toBeDefined();
  });

  it("draws an empty scale around today when no row has dates", async () => {
    const collections = createMockCollectionsStore();
    const query = collections.query.bind(collections);
    vi.spyOn(collections, "query").mockImplementation((input) =>
      query({ ...input, filter: ["start is empty"] }),
    );
    renderApp(`/data/${COLLECTION.issues}?view=Timeline`, undefined, collections);
    expect(await screen.findByRole("region", { name: "No dates" })).toBeDefined();
    expect(document.querySelector(".vl-timeline-scale")?.children).toHaveLength(5);
    expect(document.querySelector('.vl-timeline-scale [data-today="true"]')).not.toBeNull();
  });
});
