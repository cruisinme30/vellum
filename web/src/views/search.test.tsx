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
    // Every link but the title's help link is a result.
    const links = await screen.findAllByRole("link");
    const results = links.filter((link) => !link.classList.contains("vl-help-link"));
    expect(results).toHaveLength(links.length - 1);
    expect(screen.getByRole("status").textContent).toBe(`${String(results.length)} pages found`);
    expect(results[0]?.textContent).toContain("Gateway architecture");
  });

  it("reads the query from the URL and counts one result", async () => {
    renderWithProviders(<SearchView />, "/search?q=acme%20logistics");
    expect(screen.getByRole("searchbox")).toHaveProperty("value", "acme logistics");
    expect(screen.getByRole("status").textContent).toBe("Searching…");
    expect(await screen.findByText("One page found")).toBeDefined();
  });

  it("says when nothing matches, and clears back to the hint", async () => {
    const { user, location } = renderWithProviders(<SearchView />, "/search?q=zebra");
    expect(await screen.findByText("No page matches “zebra”.")).toBeDefined();
    expect(screen.getByRole("status").textContent).toBe("0 pages found");
    await user.clear(screen.getByRole("searchbox"));
    expect(location()).toBe("/search");
    expect(screen.getByRole("status").textContent).toBe(
      "Type to search every page title and body.",
    );
  });
});

describe("InboxView", () => {
  it("lists the pages that mention me", async () => {
    renderWithProviders(<InboxView />);
    expect(screen.getByRole("heading", { level: 1, name: "Inbox" })).toBeDefined();
    expect(screen.getByText("Pages that mention @priya.")).toBeDefined();
    expect(screen.getByRole("status").textContent).toBe("Searching…");
    const links = await screen.findAllByRole("link");
    expect(links.length).toBeGreaterThan(0);
  });

  it("says when nobody has mentioned me", async () => {
    const { store } = renderWithProviders(<InboxView />);
    act(() => {
      for (const page of store.getSnapshot().pages) {
        store.trashPage(page.id);
      }
    });
    expect(await screen.findByText("Nobody has mentioned you yet.")).toBeDefined();
  });
});

describe("SearchView filters and highlights", () => {
  it("marks the matched words and narrows by the filters in the URL", async () => {
    const { user, location } = renderWithProviders(<SearchView />, "/search?q=gateway");
    const results = await screen.findAllByRole("link", { name: /gateway/i });
    expect(results[0]?.querySelector("mark")?.textContent.toLowerCase()).toBe("gateway");

    await user.click(screen.getByRole("checkbox", { name: "Starred only" }));
    expect(location()).toBe("/search?q=gateway&starred=1");
    await user.selectOptions(screen.getByRole("combobox", { name: "Modified" }), "30");
    expect(location()).toBe("/search?q=gateway&starred=1&modified=30");
    await user.selectOptions(screen.getByRole("combobox", { name: "Owner" }), "me");
    expect(location()).toContain("owner=me");
    await user.selectOptions(screen.getByRole("combobox", { name: "Owner" }), "");
    expect(location()).not.toContain("owner=");
  });

  it("reads filters from the URL and leaves out pages that fail them", async () => {
    renderWithProviders(<SearchView />, "/search?q=gateway&owner=me&modified=7");
    expect(screen.getByRole("combobox", { name: "Owner" })).toHaveProperty("value", "me");
    expect(screen.getByRole("combobox", { name: "Modified" })).toHaveProperty("value", "7");
    expect(await screen.findByRole("status")).toBeDefined();
  });
});
