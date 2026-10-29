// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { NORTHSTAR, PAGE } from "../data/mock/northstar";
import { createMemoryStore } from "../data/store";
import { body, p, wiki } from "../editor/content";
import { renderApp } from "../test/render-app";

import { caretColor } from "./page-view";

async function openPage(id: string) {
  const rendered = renderApp(`/p/${id}`);
  await screen.findByRole("textbox", { name: "Page body" }, { timeout: 5000 });
  const page = () => rendered.store.getSnapshot().pages.find((p) => p.id === id);
  const title = () => screen.getByRole("textbox", { name: "Page title" });
  return { ...rendered, page, title };
}

describe("caretColor", () => {
  it("gives each handle a stable colour from the palette", () => {
    expect(caretColor("priya")).toBe(caretColor("priya"));
    expect(caretColor("")).toBe("#b3261e");
    expect(caretColor("a")).toMatch(/^#[\da-f]{6}$/);
  });
});

describe("PageView", () => {
  it("shows breadcrumbs, title and properties", async () => {
    const { page } = await openPage(PAGE.runbook);
    const crumbs = screen.getByRole("navigation", { name: "Breadcrumbs" });
    expect(
      within(crumbs)
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual(["Engineering", "Engineering home", "Gateway architecture"]);
    expect(within(crumbs).getByRole("link", { name: "Gateway architecture" })).toHaveProperty(
      "pathname",
      `/p/${PAGE.gateway}`,
    );
    expect(screen.getByRole("textbox", { name: "Page title" })).toHaveProperty(
      "value",
      "Runbook: deploy the gateway",
    );
    const properties = screen.getAllByRole("definition").map((item) => item.textContent);
    expect(properties[0]).toBe("Verified");
    expect(properties[1]).toBe("Sam Okafor");
    expect(screen.getByRole("time").getAttribute("datetime")).toBe(page()?.updatedAt);
  });

  it("renames the page when Enter leaves the title", async () => {
    const { user, page, title } = await openPage(PAGE.launch);
    await user.clear(title());
    await user.type(title(), "  Launch plan  {Enter}");
    expect(page()?.title).toBe("Launch plan");
    expect(document.activeElement).not.toBe(title());
    expect(screen.getByRole("treeitem", { name: "Launch plan" })).toBeDefined();
  });

  it("leaves an unchanged title alone", async () => {
    const { user, page, title } = await openPage(PAGE.launch);
    const before = page()?.updatedAt;
    await user.click(title());
    await user.type(title(), "{End}{Shift}");
    await user.tab();
    expect(page()?.updatedAt).toBe(before);
  });

  it("stars and unstars the page", async () => {
    const { user, page } = await openPage(PAGE.prd);
    const star = screen.getByRole("button", { name: "Favorite" });
    expect(star.getAttribute("aria-pressed")).toBe("true");
    await user.click(star);
    expect(star.getAttribute("aria-pressed")).toBe("false");
    expect(page()?.starred).toBe(false);
  });

  it("links pages named in the body", async () => {
    await openPage(PAGE.prd);
    const body = screen.getByRole("textbox", { name: "Page body" });
    expect(within(body).getByRole("link", { name: /Gateway architecture/ })).toHaveProperty(
      "pathname",
      `/p/${PAGE.gateway}`,
    );
  });

  it("resolves links by page id and marks unknown pages; shows unknown owners by handle", async () => {
    const store = createMemoryStore({
      ...NORTHSTAR,
      pages: NORTHSTAR.pages.map((page) =>
        page.id === PAGE.launch ? { ...page, owner: "ghost" } : page,
      ),
      bodies: {
        [PAGE.launch]: body(p(wiki(`vellum://doc/${PAGE.engHome}`), " ", wiki("Nowhere"))),
      },
    });
    renderApp(`/p/${PAGE.launch}`, store);
    const editor = await screen.findByRole("textbox", { name: "Page body" }, { timeout: 5000 });
    expect(within(editor).getByRole("link", { name: /Engineering home/ })).toHaveProperty(
      "pathname",
      `/p/${PAGE.engHome}`,
    );
    expect(within(editor).getByRole("link", { name: /Nowhere/ }).className).toContain("vl-missing");
    expect(screen.getAllByRole("definition")[1]?.textContent).toBe("ghost");
  });

  it("names untitled ancestors", async () => {
    const { store } = await openPage(PAGE.acme);
    act(() => {
      store.renamePage(PAGE.interviews, "");
    });
    const crumbs = screen.getByRole("navigation", { name: "Breadcrumbs" });
    expect(within(crumbs).getByRole("link", { name: "Untitled" })).toBeDefined();
  });

  it("shows not found for a missing page", () => {
    renderApp("/p/NOPE");
    return expect(
      screen.findByRole("heading", { level: 1, name: "This page does not exist" }),
    ).resolves.toBeDefined();
  });
});
