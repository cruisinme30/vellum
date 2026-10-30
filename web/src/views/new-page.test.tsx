// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { createNorthstarStore } from "../data/mock/northstar";
import { renderApp } from "../test/render-app";

describe("NewPageView", () => {
  it(
    "creates one page in the first space and opens it",
    { timeout: 15_000 },
    async () => {
      const store = createNorthstarStore();
      const before = store.getSnapshot().pages.length;
      const { location } = renderApp("/new?source=pwa-shortcut", store);
      // The page view loads lazily, which is slow under coverage.
      await screen.findByRole(
        "textbox",
        { name: "Page title" },
        { timeout: 10_000 },
      );
      expect(location()).toMatch(/^\/p\//);
      expect(store.getSnapshot().pages).toHaveLength(before + 1);
    },
  );

  it("serves the My Work shortcut", () => {
    renderApp("/my-work");
    expect(screen.getByRole("heading", { level: 1 }).textContent).not.toBe("");
  });
});
