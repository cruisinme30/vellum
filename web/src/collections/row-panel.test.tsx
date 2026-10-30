// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, waitFor, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { renderApp } from "../test/render-app";

import { createMockCollectionsStore } from "./mock";
import { COLLECTION } from "./mock-data";
import { cellValue } from "./value";

async function openPanel(title = "Token bucket per tenant") {
  const collections = createMockCollectionsStore();
  const rendered = renderApp(
    `/data/${COLLECTION.issues}?view=Gallery`,
    undefined,
    collections,
  );
  const card = await screen.findByRole("button", {
    name: new RegExp(`^${title}`),
  });
  await rendered.user.click(card);
  const panel = await screen.findByRole("dialog", { name: title });
  await within(panel).findByRole("textbox", { name: "Issue" });
  const row = async (key: string) => {
    const { groups } = await collections.query({
      collection_id: COLLECTION.issues,
    });
    const found = groups[0]?.rows.find((r) =>
      r.title.startsWith(title.slice(0, 6)),
    );
    const detail = await collections.getRow({
      collection_id: COLLECTION.issues,
      row_id: found?.id ?? "",
    });
    return cellValue(detail.row.cells[key]);
  };
  return { ...rendered, card, panel, row };
}

describe("RowPanel", () => {
  it("shows every property, with computed ones read-only", async () => {
    const { panel } = await openPanel();
    expect(panel.getAttribute("aria-modal")).toBe("false");
    expect(
      within(panel).getByRole("textbox", { name: "Issue" }),
    ).toHaveProperty("value", "Token bucket per tenant");
    expect(
      within(panel).getByRole("button", { name: "Status In progress" }),
    ).toBeDefined();
    expect(
      within(panel).getByRole("button", { name: "Owner Dev Rao" }),
    ).toBeDefined();
    expect(
      within(panel).getByRole("button", { name: "Release Gateway beta" }),
    ).toBeDefined();
    expect(within(panel).getByLabelText("Due")).toHaveProperty(
      "value",
      "2026-11-13",
    );
    expect(
      within(panel).getByRole("checkbox", { name: "Blocked" }),
    ).toHaveProperty("checked", false);
    expect(
      within(panel).getByRole("status", { name: "Points left" }).textContent,
    ).toBe("5");
    expect(within(panel).getByText("Computed, read-only")).toBeDefined();
    expect(
      within(panel).getByRole("textbox", { name: "Notes" }),
    ).toHaveProperty("value", "Notes on token bucket per tenant.");
    expect(document.activeElement).toBe(
      within(panel).getByRole("button", { name: "Close row" }),
    );
  });

  it(
    "saves text on Enter or when leaving the field, and explains bad " +
      "input",
    async () => {
      const { user, panel, row } = await openPanel();
      const title = within(panel).getByRole("textbox", { name: "Issue" });
      await user.clear(title);
      await user.type(title, "Token buckets{Enter}");
      expect(
        await screen.findByRole("heading", { level: 2, name: "Token buckets" }),
      ).toBeDefined();
      const estimate = within(panel).getByRole("textbox", { name: "Estimate" });
      await user.clear(estimate);
      await user.type(estimate, "five");
      await user.tab();
      expect(within(panel).getByRole("alert").textContent).toBe(
        "Enter a number, such as 3 or 2.5.",
      );
      expect(estimate.getAttribute("aria-invalid")).toBe("true");
      await user.clear(estimate);
      await user.type(estimate, "1,234");
      await user.tab();
      await waitFor(async () => {
        expect(await row("est")).toEqual({ type: "number", value: 1234 });
      });
      const link = within(panel).getByRole("textbox", { name: "Link" });
      await user.type(link, "not a url{Enter}");
      expect(within(panel).getByRole("alert").textContent).toBe(
        "Enter a full address, such as https://example.com.",
      );
    },
  );

  it("picks options, people and related rows", async () => {
    const { user, panel, row } = await openPanel();
    await user.click(
      within(panel).getByRole("button", { name: "Status In progress" }),
    );
    await user.click(within(panel).getByRole("option", { name: "Done" }));
    expect(
      await within(panel).findByRole("button", { name: "Status Done" }),
    ).toBeDefined();
    expect(
      within(panel).getByRole("status", { name: "Points left" }).textContent,
    ).toBe("0");

    await user.click(
      within(panel).getByRole("button", { name: "Labels feature" }),
    );
    await user.click(within(panel).getByRole("option", { name: "bug" }));
    await user.keyboard("{Enter}");
    expect(
      await within(panel).findByRole("button", { name: "Labels bug feature" }),
    ).toBeDefined();

    await user.click(
      within(panel).getByRole("button", { name: "Owner Dev Rao" }),
    );
    await user.click(within(panel).getByRole("option", { name: "None" }));
    expect(
      await within(panel).findByRole("button", { name: "Owner None" }),
    ).toBeDefined();
    expect(await row("owner")).toEqual({ type: "empty" });

    await user.click(
      within(panel).getByRole("button", { name: "Release Gateway beta" }),
    );
    await user.click(
      await within(panel).findByRole("option", { name: "Gateway 1.1" }),
    );
    await user.keyboard("{Escape}");
    expect(
      within(panel).getByRole("button", { name: "Release Gateway beta" }),
    ).toBeDefined();
    await user.click(
      within(panel).getByRole("button", { name: "Release Gateway beta" }),
    );
    await user.click(
      await within(panel).findByRole("option", { name: "Gateway 1.1" }),
    );
    await user.click(within(panel).getByRole("textbox", { name: "Notes" }));
    expect(
      await within(panel).findByRole("button", {
        name: "Release Gateway beta Gateway 1.1",
      }),
    ).toBeDefined();
  });

  it("toggles a checkbox and saves the notes", async () => {
    const { user, panel, row } = await openPanel();
    await user.click(within(panel).getByRole("checkbox", { name: "Blocked" }));
    await waitFor(() => {
      expect(
        within(panel).getByRole("checkbox", { name: "Blocked" }),
      ).toHaveProperty("checked", true);
    });
    const notes = within(panel).getByRole("textbox", { name: "Notes" });
    await user.type(notes, " More.");
    await user.tab();
    await waitFor(() => {
      expect(
        within(panel).getByRole("textbox", { name: "Notes" }),
      ).toHaveProperty("value", "Notes on token bucket per tenant. More.");
    });
    expect(await row("blocked")).toEqual({ type: "bool", value: true });
    // Leaving unchanged notes writes nothing.
    await user.click(within(panel).getByRole("textbox", { name: "Notes" }));
    await user.tab();
  });

  it("shows refused writes next to the field", async () => {
    const collections = createMockCollectionsStore();
    const { user } = renderApp(
      `/data/${COLLECTION.issues}?view=Gallery`,
      undefined,
      collections,
    );
    await user.click(
      await screen.findByRole("button", { name: /^Token bucket/ }),
    );
    const panel = await screen.findByRole("dialog");
    await within(panel).findByRole("checkbox", { name: "Blocked" });
    vi.spyOn(collections, "updateRow").mockRejectedValue(
      new Error("read-only workspace"),
    );
    await user.click(within(panel).getByRole("checkbox", { name: "Blocked" }));
    expect(await within(panel).findByRole("alert")).toHaveProperty(
      "textContent",
      "read-only workspace",
    );
    await user.type(within(panel).getByRole("textbox", { name: "Notes" }), "!");
    await user.tab();
    await waitFor(() => {
      expect(within(panel).getAllByRole("alert")).toHaveLength(2);
    });
    await user.click(
      within(panel).getByRole("button", { name: "Status In progress" }),
    );
    await user.click(within(panel).getByRole("option", { name: "Done" }));
    await waitFor(() => {
      expect(within(panel).getAllByRole("alert")).toHaveLength(3);
    });
  });

  it("closes with Escape or its button and gives focus back", async () => {
    const { user, card } = await openPanel();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(card);
    await user.click(card);
    await user.click(await screen.findByRole("button", { name: "Close row" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("shows a row that cannot be read", async () => {
    const collections = createMockCollectionsStore();
    vi.spyOn(collections, "getRow").mockRejectedValue(new Error("row gone"));
    const { user } = renderApp(
      `/data/${COLLECTION.issues}?view=Gallery`,
      undefined,
      collections,
    );
    await user.click(
      await screen.findByRole("button", { name: /^Token bucket/ }),
    );
    const panel = await screen.findByRole("dialog", { name: "Untitled" });
    expect(await within(panel).findByRole("alert")).toHaveProperty(
      "textContent",
      "row gone",
    );
  });

  it("shows formula errors and percentages", async () => {
    const collections = createMockCollectionsStore();
    const getRow = collections.getRow.bind(collections);
    const info = collections.info.bind(collections);
    vi.spyOn(collections, "info").mockImplementation(async (id) => {
      const found = await info(id);
      return {
        ...found,
        properties: found.properties.map((p) =>
          p.key === "left" ? { ...p, format: "percent" as const } : p,
        ),
      };
    });
    vi.spyOn(collections, "getRow").mockImplementation(async (key) => {
      const detail = await getRow(key);
      const left = detail.title.startsWith("Token")
        ? { value: { type: "number", value: 0.5 } as const }
        : { error: { message: "division by zero" } };
      return {
        ...detail,
        row: { ...detail.row, cells: { ...detail.row.cells, left } },
      };
    });
    const { user } = renderApp(
      `/data/${COLLECTION.issues}?view=Gallery`,
      undefined,
      collections,
    );
    await user.click(
      await screen.findByRole("button", { name: /^Token bucket/ }),
    );
    const panel = await screen.findByRole("dialog", {
      name: "Token bucket per tenant",
    });
    expect(
      await within(panel).findByRole("status", { name: "Points left" }),
    ).toHaveProperty("textContent", "50%");
    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: /^Gateway returns/ }));
    const other = await screen.findByRole("dialog", {
      name: /Gateway returns/,
    });
    expect(
      await within(other).findByRole("status", { name: "Points left" }),
    ).toHaveProperty("textContent", "⚠ division by zero");
  });
});
