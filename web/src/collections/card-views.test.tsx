// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { renderApp } from "../test/render-app";

import { createMockCollectionsStore } from "./mock";
import { COLLECTION } from "./mock-data";
import type { QueryInput } from "./types";

/** A store whose queries are changed on the way out. */
function storeWith(change: (input: QueryInput) => Partial<QueryInput>, empty = false) {
  const collections = createMockCollectionsStore();
  const query = collections.query.bind(collections);
  vi.spyOn(collections, "query").mockImplementation(async (input) => {
    const result = await query({ ...input, ...change(input) });
    return empty ? { ...result, groups: [] } : result;
  });
  return collections;
}

describe("GalleryView", () => {
  it("shows a card per row with its first values, and opens it", async () => {
    const { user } = renderApp(`/data/${COLLECTION.issues}?view=Gallery`);
    const gallery = await screen.findByRole("list", { name: "Gallery" });
    const cards = within(gallery).getAllByRole("button");
    expect(cards).toHaveLength(8);
    const first = cards[0];
    expect(first?.textContent).toMatch(/^TToken bucket per tenantStatusIn progressLabelsfeature/);
    expect(first?.querySelectorAll(".vl-card-field")).toHaveLength(4);
    await user.click(cards[1] ?? gallery);
    expect(
      await screen.findByRole("dialog", { name: "Gateway returns 502 on cold start" }),
    ).toBeDefined();
  });

  it("says when there are no rows", async () => {
    renderApp(
      `/data/${COLLECTION.issues}?view=Gallery`,
      undefined,
      storeWith(() => ({}), true),
    );
    expect(await screen.findByText("No rows match this view.")).toBeDefined();
  });
});

describe("ListView", () => {
  it("lists rows compactly in the view's order", async () => {
    const { user } = renderApp(`/data/${COLLECTION.issues}?view=Open`);
    const list = await screen.findByRole("region", { name: "Open" });
    const rows = within(list).getAllByRole("button");
    expect(rows.map((row) => row.querySelector(".vl-card-title")?.textContent)).toEqual([
      "Replay protection for webhooks",
      "Token bucket per tenant",
      "Gateway returns 502 on cold start",
      "Document rate-limit headers",
      "Runbook: drain a region",
      "Flaky TLS handshake test",
    ]);
    expect(rows[0]?.querySelectorAll(".vl-card-field")).toHaveLength(3);
    expect(within(list).queryByRole("heading")).toBeNull();
    await user.click(rows[0] ?? list);
    expect(await screen.findByRole("dialog")).toBeDefined();
  });

  it("puts grouped rows under their group's heading", async () => {
    renderApp(
      `/data/${COLLECTION.issues}?view=Open`,
      undefined,
      storeWith(() => ({ group: "status" })),
    );
    expect(await screen.findByRole("heading", { level: 2, name: "Backlog" })).toBeDefined();
    const progress = screen.getByRole("region", { name: "In progress" });
    expect(within(progress).getAllByRole("button")).toHaveLength(2);
  });

  it("says when there are no rows", async () => {
    renderApp(
      `/data/${COLLECTION.issues}?view=Open`,
      undefined,
      storeWith(() => ({}), true),
    );
    expect(await screen.findByText("No rows match this view.")).toBeDefined();
  });
});
