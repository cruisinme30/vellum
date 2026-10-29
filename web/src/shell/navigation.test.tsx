// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { matchQueries, renderApp } from "../test/render-app";

import { COMPACT, MEDIUM } from "./use-media";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("NavigationRail", () => {
  it("links every destination and marks the current one", () => {
    renderApp("/inbox");
    const rail = screen.getByRole("navigation", { name: "Main" });
    const links = within(rail).getAllByRole("link");
    expect(links.map((link) => [link.textContent, link.getAttribute("href")])).toEqual([
      ["Home", "/"],
      ["Search", "/search"],
      ["Favorites", "/favorites"],
      ["Inbox", "/inbox"],
      ["Data", "/data"],
      ["Help", "/help"],
      ["Admin", "/admin"],
    ]);
    expect(within(rail).getByRole("link", { name: "Inbox" }).getAttribute("aria-current")).toBe(
      "page",
    );
    expect(within(rail).getByRole("link", { name: "Home" }).hasAttribute("aria-current")).toBe(
      false,
    );
  });

  it("navigates to a destination", async () => {
    const { user, location } = renderApp("/");
    await user.click(screen.getByRole("link", { name: "Help" }));
    expect(location()).toBe("/help");
    expect(screen.getByRole("heading", { level: 1, name: "Help" })).toBeDefined();
  });

  it("opens and closes the theme settings", async () => {
    const { user } = renderApp("/");
    const button = screen.getByRole("button", { name: "Theme" });
    expect(button.getAttribute("aria-haspopup")).toBe("dialog");
    await user.click(button);
    expect(button.getAttribute("aria-expanded")).toBe("true");
    const dialog = screen.getByRole("dialog", { name: "Theme" });
    expect(within(dialog).getByRole("radio", { name: "Dark" })).toBeDefined();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(button.getAttribute("aria-expanded")).toBe("false");
  });

  it("hides and shows the pages sidebar", async () => {
    const { user } = renderApp("/");
    const toggle = screen.getByRole("button", { name: "Hide pages" });
    expect(toggle.getAttribute("aria-controls")).toBe("vl-sidebar");
    expect(screen.getByRole("complementary", { name: "Pages" })).toBeDefined();
    await user.click(toggle);
    expect(screen.queryByRole("complementary", { name: "Pages" })).toBeNull();
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    await user.click(screen.getByRole("button", { name: "Show pages" }));
    expect(screen.getByRole("complementary", { name: "Pages" })).toBeDefined();
  });
});

describe("BottomBar", () => {
  it("replaces the rail on phones", () => {
    vi.stubGlobal("matchMedia", matchQueries(COMPACT, MEDIUM));
    renderApp("/");
    const bar = screen.getByRole("navigation", { name: "Main" });
    expect(bar.className).toBe("vl-bottom-bar");
    expect(
      within(bar)
        .getAllByRole("link")
        .map((link) => link.textContent),
    ).toEqual(["Home", "Search", "Inbox"]);
    expect(screen.queryByRole("button", { name: "Hide pages" })).toBeNull();
    expect(screen.queryByRole("complementary", { name: "Pages" })).toBeNull();
  });

  it("opens the More sheet with the other destinations and theme", async () => {
    vi.stubGlobal("matchMedia", matchQueries(COMPACT, MEDIUM));
    const { user, location } = renderApp("/");
    const more = screen.getByRole("button", { name: "More" });
    expect(more.getAttribute("aria-expanded")).toBe("false");
    await user.click(more);
    expect(more.getAttribute("aria-expanded")).toBe("true");
    const sheet = screen.getByRole("complementary", { name: "Pages" });
    const links = within(sheet).getByRole("navigation", { name: "More destinations" });
    expect(
      within(links)
        .getAllByRole("link")
        .map((link) => link.textContent),
    ).toEqual(["Favorites", "Data", "Help", "Admin"]);
    expect(within(sheet).getByRole("region", { name: "Theme" })).toBeDefined();
    await user.click(within(links).getByRole("link", { name: "Help" }));
    expect(location()).toBe("/help");
    expect(screen.queryByRole("complementary", { name: "Pages" })).toBeNull();
  });

  it("closes the More sheet with its button or the scrim", async () => {
    vi.stubGlobal("matchMedia", matchQueries(COMPACT, MEDIUM));
    const { user, container } = renderApp("/");
    const more = screen.getByRole("button", { name: "More" });
    await user.click(more);
    await user.click(more);
    expect(screen.queryByRole("complementary", { name: "Pages" })).toBeNull();
    await user.click(more);
    const scrim = container.querySelector(".vl-scrim");
    expect(scrim).not.toBeNull();
    if (scrim) {
      await user.click(scrim);
    }
    expect(screen.queryByRole("complementary", { name: "Pages" })).toBeNull();
  });

  it("creates a page from New", async () => {
    vi.stubGlobal("matchMedia", matchQueries(COMPACT, MEDIUM));
    const { user, store, location } = renderApp("/help");
    await user.click(screen.getByRole("button", { name: "New" }));
    const created = store.getSnapshot().pages.at(-1);
    expect(created?.title).toBe("");
    expect(location()).toBe(`/p/${created?.id ?? ""}`);
  });
});
