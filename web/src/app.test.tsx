// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { App } from "./app";
import { createNorthstarStore, PAGE } from "./data/mock/northstar";
import { createTestCollab, renderApp } from "./test/render-app";

describe("AppRoutes", () => {
  it.each([
    ["/", "Good to see you, Priya"],
    ["/search", "Search"],
    ["/favorites", "Favorites"],
    ["/inbox", "Inbox"],
    ["/data", "Data"],
    ["/help", "Help"],
    ["/admin", "Admin"],
    ["/no/such/route", "This page does not exist"],
  ])("renders the view of %s", (path, heading) => {
    renderApp(path);
    expect(screen.getByRole("heading", { level: 1, name: heading })).toBeDefined();
    expect(screen.getByRole("main")).toBeDefined();
  });

  it("shows a loading status, then the page editor", async () => {
    renderApp(`/p/${PAGE.gateway}`);
    expect(screen.getByRole("status").textContent).toBe("Opening the page…");
    expect(
      await screen.findByRole("textbox", { name: "Page body" }, { timeout: 5000 }),
    ).toBeDefined();
    expect(screen.getByRole("textbox", { name: "Page title" })).toHaveProperty(
      "value",
      "Gateway architecture",
    );
  });
});

describe("App", () => {
  it("renders the shell in a browser router", () => {
    render(<App store={createNorthstarStore()} collab={createTestCollab()} />);
    expect(screen.getByRole("navigation", { name: "Main" })).toBeDefined();
    expect(screen.getByRole("heading", { level: 1, name: "Good to see you, Priya" })).toBeDefined();
  });
});
