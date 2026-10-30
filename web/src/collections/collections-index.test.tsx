// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { renderApp } from "../test/render-app";

import { createMockCollectionsStore } from "./mock";
import { COLLECTION } from "./mock-data";

describe("CollectionsIndexView", () => {
  it("lists every collection with its size, linking to it", async () => {
    const { user, location } = renderApp("/data");
    expect(
      screen.getByRole("heading", { level: 1, name: "Data" }),
    ).toBeDefined();
    expect(screen.getByRole("status").textContent).toBe("Loading…");
    const issues = await screen.findByRole("link", {
      name: "Issues 8 rows · 6 views",
    });
    expect(
      screen.getByRole("link", { name: "Releases 3 rows · one view" }),
    ).toBeDefined();
    await user.click(issues);
    expect(location()).toBe(`/data/${COLLECTION.issues}`);
    expect(
      await screen.findByRole("heading", { level: 1, name: "Issues" }),
    ).toBeDefined();
  });

  it("says when there are no collections", async () => {
    const collections = createMockCollectionsStore();
    vi.spyOn(collections, "list").mockResolvedValue({
      collections: [
        { id: "x", title: "Solo", path: "solo.collection", rows: 1, views: [] },
      ],
    });
    renderApp("/data", undefined, collections);
    expect(
      await screen.findByRole("link", { name: "Solo One row · 0 views" }),
    ).toBeDefined();
    vi.spyOn(collections, "list").mockResolvedValue({ collections: [] });
    renderApp("/data", undefined, collections);
    expect(
      await screen.findByText("There are no collections yet."),
    ).toBeDefined();
  });

  it("says when the list cannot be read", async () => {
    const collections = createMockCollectionsStore();
    vi.spyOn(collections, "list").mockRejectedValue("offline");
    renderApp("/data", undefined, collections);
    expect(await screen.findByRole("alert")).toHaveProperty(
      "textContent",
      "offline",
    );
  });
});
