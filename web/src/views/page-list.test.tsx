// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { NORTHSTAR } from "../data/mock/northstar";
import type { PageMeta } from "../data/types";
import { renderWithProviders } from "../test/render-app";

import { PageList } from "./page-list";

const [first, second] = NORTHSTAR.pages as [PageMeta, PageMeta];

describe("PageList", () => {
  it("shows the empty message when there are no pages", () => {
    renderWithProviders(<PageList items={[]} empty="Nothing here." />);
    expect(screen.getByText("Nothing here.")).toBeDefined();
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("links each page with its snippet, if any", () => {
    renderWithProviders(
      <PageList
        items={[
          { page: first, snippet: "…the gateway…" },
          { page: second, snippet: "" },
        ]}
        empty="Nothing here."
      />,
    );
    const links = screen.getAllByRole("link");
    expect(links.map((link) => link.getAttribute("href"))).toEqual([
      `/p/${first.id}`,
      `/p/${second.id}`,
    ]);
    expect(links[0]?.textContent).toBe(`${first.icon}${first.title}…the gateway…`);
    expect(links[1]?.textContent).toBe(`${second.icon}${second.title}`);
  });

  it("names untitled pages", () => {
    renderWithProviders(<PageList items={[{ page: { ...first, title: "" } }]} empty="" />);
    expect(screen.getByRole("link", { name: "Untitled" })).toBeDefined();
  });
});
