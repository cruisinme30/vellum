// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { SLASH_ITEMS } from "../editor/slash/items";
import { renderWithProviders } from "../test/render-app";

import { AdminView, HelpView, NotFoundView, PendingView } from "./info";

describe("HelpView", () => {
  it("lists the keyboard shortcuts", () => {
    renderWithProviders(<HelpView />);
    expect(screen.getByRole("heading", { level: 2, name: "Keyboard shortcuts" })).toBeDefined();
    const terms = screen.getAllByRole("term").map((term) => term.textContent);
    expect(terms).toContain("Shift F10");
    expect(screen.getAllByRole("definition")).toHaveLength(terms.length);
    expect(screen.getByText("Reorder and nest pages in the page tree")).toBeDefined();
  });

  it("lists every block the slash menu inserts", () => {
    renderWithProviders(<HelpView />);
    expect(
      screen.getByRole("heading", { level: 2, name: "Blocks you can insert with /" }),
    ).toBeDefined();
    expect(screen.getAllByRole("listitem")).toHaveLength(SLASH_ITEMS.length);
  });
});

describe("pending views", () => {
  it("shows a title and an explanation", () => {
    renderWithProviders(<PendingView title="Later" body="Coming soon." />);
    expect(screen.getByRole("heading", { level: 1, name: "Later" })).toBeDefined();
    expect(screen.getByText("Coming soon.")).toBeDefined();
  });

  it("lists the admin screens", () => {
    renderWithProviders(<AdminView />);
    expect(screen.getByRole("heading", { level: 1, name: "Admin" })).toBeDefined();
    const screens: [string, string][] = [
      ["Audit log", "/admin/audit"],
      ["Settings", "/admin/settings"],
      ["Backups", "/admin/backups"],
      ["Requests", "/admin/requests"],
      ["Routes", "/admin/routes"],
    ];
    for (const [name, path] of screens) {
      expect(screen.getByRole("link", { name }).getAttribute("href")).toBe(path);
    }
  });
});

describe("NotFoundView", () => {
  it("links back home", async () => {
    const { user, location } = renderWithProviders(<NotFoundView />, "/missing");
    expect(
      screen.getByRole("heading", { level: 1, name: "This page does not exist" }),
    ).toBeDefined();
    await user.click(screen.getByRole("link", { name: "Go home" }));
    expect(location()).toBe("/");
  });
});
