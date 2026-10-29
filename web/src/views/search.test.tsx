// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { renderWithProviders } from "../test/render-app";

import { InboxView, SearchView } from "./search";

describe("SearchView", () => {
  it("focuses the search field and explains what it searches", () => {
    renderWithProviders(<SearchView />, "/search");
    expect(screen.getByRole("search")).toBeDefined();
    expect(document.activeElement).toBe(screen.getByRole("searchbox", { name: "Search pages" }));
    expect(screen.getByRole("status").textContent).toBe(
      "Type to search every page title and body.",
    );
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("keeps the query in the URL and lists matching pages", async () => {
    const { user, location } = renderWithProviders(<SearchView />, "/search");
    await user.type(screen.getByRole("searchbox"), "gateway");
    expect(location()).toBe("/search?q=gateway");
    const results = screen.getAllByRole("link");
    expect(screen.getByRole("status").textContent).toBe(`${String(results.length)} pages found`);
    expect(results[0]?.textContent).toContain("Gateway architecture");
  });

  it("reads the query from the URL and counts one result", () => {
    renderWithProviders(<SearchView />, "/search?q=acme%20logistics");
    expect(screen.getByRole("searchbox")).toHaveProperty("value", "acme logistics");
    expect(screen.getByRole("status").textContent).toBe("One page found");
  });

  it("says when nothing matches, and clears back to the hint", async () => {
    const { user, location } = renderWithProviders(<SearchView />, "/search?q=zebra");
    expect(screen.getByRole("status").textContent).toBe("0 pages found");
    expect(screen.getByText("No page matches “zebra”.")).toBeDefined();
    await user.clear(screen.getByRole("searchbox"));
    expect(location()).toBe("/search");
    expect(screen.getByRole("status").textContent).toBe(
      "Type to search every page title and body.",
    );
  });
});

describe("InboxView", () => {
  it("lists the pages that mention me", () => {
    renderWithProviders(<InboxView />);
    expect(screen.getByRole("heading", { level: 1, name: "Inbox" })).toBeDefined();
    expect(screen.getByText("Pages that mention @priya.")).toBeDefined();
    expect(screen.getAllByRole("link").length).toBeGreaterThan(0);
  });

  it("says when nobody has mentioned me", () => {
    const { store } = renderWithProviders(<InboxView />);
    act(() => {
      for (const page of store.getSnapshot().pages) {
        store.trashPage(page.id);
      }
    });
    expect(screen.getByText("Nobody has mentioned you yet.")).toBeDefined();
  });
});
