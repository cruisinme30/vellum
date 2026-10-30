// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { renderApp } from "../test/render-app";

import { createMockCollectionsStore } from "./mock";
import { COLLECTION } from "./mock-data";

async function renderTable() {
  const collections = createMockCollectionsStore();
  const rendered = renderApp(
    `/data/${COLLECTION.issues}`,
    undefined,
    collections,
  );
  const grid = await screen.findByRole("grid", { name: "All issues" });
  const rows = () => within(grid).getAllByRole("row");
  const cell = (row: number, column: number) => {
    const found = within(rows()[row] ?? grid).getAllByRole("gridcell")[column];
    if (!found) {
      throw new Error(`no cell ${String(row)}:${String(column)}`);
    }
    return found;
  };
  const focus = (row: number, column: number) => {
    act(() => {
      cell(row, column).focus();
    });
  };
  const query = () => collections.query({ collection_id: COLLECTION.issues });
  return { ...rendered, grid, rows, cell, focus, query };
}

// Columns: Issue, Status, Labels, Owner, Start, Due, Estimate, Blocked, Link,
// Release, Points left.
const [STATUS, LABELS, OWNER, START, ESTIMATE, BLOCKED, LINK, RELEASE, LEFT] = [
  1, 2, 3, 4, 6, 7, 8, 9, 10,
];

describe("TableView", () => {
  it("is a grid with a header row, counts and indexes", async () => {
    const { grid, rows, cell } = await renderTable();
    expect(grid.getAttribute("aria-rowcount")).toBe("9");
    expect(grid.getAttribute("aria-colcount")).toBe("11");
    expect(grid.getAttribute("aria-describedby")).not.toBeNull();
    const headers = within(grid)
      .getAllByRole("columnheader")
      .map((h) => h.textContent);
    expect(headers).toEqual([
      "Issue",
      "Status",
      "Labels",
      "Owner",
      "Start",
      "Due",
      "Estimate",
      "Blocked",
      "Link",
      "Release",
      "Points left",
    ]);
    expect(rows()[1]?.getAttribute("aria-rowindex")).toBe("2");
    expect(cell(1, 0).textContent).toContain("Trace sampling knobs");
    expect(cell(1, 0).getAttribute("aria-colindex")).toBe("1");
    expect(cell(1, 0).tabIndex).toBe(0);
    expect(cell(1, 1).tabIndex).toBe(-1);
    expect(cell(1, OWNER).textContent).toBe("Dev Rao");
    expect(cell(1, BLOCKED).textContent).toBe("No");
    expect(cell(1, LEFT).getAttribute("aria-readonly")).toBe("true");
    expect(cell(3, LINK).textContent).toBe(
      "https://status.northstar.example/incidents/42",
    );
    expect(screen.getByText(/Arrow keys move between cells/)).toBeDefined();
  });

  it("moves the active cell with arrows, Home, End and Page keys", async () => {
    const { user, cell, focus } = await renderTable();
    focus(1, 0);
    await user.keyboard("{ArrowRight}");
    expect(document.activeElement).toBe(cell(1, 1));
    expect(cell(1, 1).tabIndex).toBe(0);
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(document.activeElement).toBe(cell(3, 1));
    await user.keyboard("{ArrowLeft}{ArrowLeft}{ArrowUp}");
    expect(document.activeElement).toBe(cell(2, 0));
    await user.keyboard("{End}");
    expect(document.activeElement).toBe(cell(2, LEFT));
    await user.keyboard("{Home}");
    expect(document.activeElement).toBe(cell(2, 0));
    await user.keyboard("{Control>}{End}{/Control}");
    expect(document.activeElement).toBe(cell(8, LEFT));
    await user.keyboard("{Control>}{Home}{/Control}");
    expect(document.activeElement).toBe(cell(1, 0));
    await user.keyboard("{PageDown}");
    expect(document.activeElement).toBe(cell(8, 0));
    await user.keyboard("{PageUp}{ArrowUp}");
    expect(document.activeElement).toBe(cell(1, 0));
    await user.keyboard("x");
    expect(document.activeElement).toBe(cell(1, 0));
  });

  it("edits text with Enter, keeps it with Enter and drops it with Escape", async () => {
    const { user, cell, focus, query } = await renderTable();
    focus(1, 0);
    await user.keyboard("{Enter}");
    const input = within(cell(1, 0)).getByRole("textbox", {
      name: "Issue of Trace sampling knobs",
    });
    expect(document.activeElement).toBe(input);
    await user.clear(input);
    await user.type(input, "Trace sampling controls{Enter}");
    expect(
      await within(cell(1, 0)).findByText("Trace sampling controls"),
    ).toBeDefined();
    expect(document.activeElement).toBe(cell(1, 0));
    const result = await query();
    expect(result.groups[0]?.rows[0]?.title).toBe("Trace sampling controls");

    await user.keyboard("{F2}");
    await user.keyboard("{Control>}a{/Control}Nope{Escape}");
    expect(within(cell(1, 0)).queryByRole("textbox")).toBeNull();
    expect(cell(1, 0).textContent).toContain("Trace sampling controls");
    expect(document.activeElement).toBe(cell(1, 0));
  });

  it("explains a number it cannot read and keeps the editor open", async () => {
    const { user, cell, focus } = await renderTable();
    focus(1, ESTIMATE);
    await user.keyboard("{Enter}");
    const input = within(cell(1, ESTIMATE)).getByRole("textbox");
    expect(input.getAttribute("inputmode")).toBe("decimal");
    await user.clear(input);
    await user.type(input, "lots{Enter}");
    expect(within(cell(1, ESTIMATE)).getByRole("alert").textContent).toBe(
      "Enter a number, such as 3 or 2.5.",
    );
    expect(input.getAttribute("aria-invalid")).toBe("true");
    await user.clear(input);
    await user.type(input, "8{Enter}");
    expect(await within(cell(1, ESTIMATE)).findByText("8")).toBeDefined();
  });

  it("keeps an edit when focus leaves, and drops one that does not parse", async () => {
    const { user, cell, focus } = await renderTable();
    focus(1, ESTIMATE);
    await user.keyboard("{Enter}");
    await user.clear(within(cell(1, ESTIMATE)).getByRole("textbox"));
    await user.type(within(cell(1, ESTIMATE)).getByRole("textbox"), "13");
    await user.click(cell(2, 0));
    expect(await within(cell(1, ESTIMATE)).findByText("13")).toBeDefined();

    await user.dblClick(cell(2, ESTIMATE));
    await user.type(within(cell(2, ESTIMATE)).getByRole("textbox"), "x");
    await user.click(cell(3, 0));
    expect(within(cell(2, ESTIMATE)).queryByRole("textbox")).toBeNull();
    expect(cell(2, ESTIMATE).textContent).toBe("5");
  });

  it("edits a date", async () => {
    const { user, cell, focus } = await renderTable();
    focus(1, START);
    await user.keyboard("{Enter}");
    const input = within(cell(1, START)).getByLabelText(
      "Start of Trace sampling knobs",
    );
    expect(input.getAttribute("type")).toBe("date");
    expect(input).toHaveProperty("value", "2026-10-26");
    await user.clear(input);
    await user.type(input, "2026-10-20{Enter}");
    expect(await within(cell(1, START)).findByText("2026-10-20")).toBeDefined();
  });

  it("picks a select option from a listbox", async () => {
    const { user, cell, focus } = await renderTable();
    focus(1, STATUS);
    await user.keyboard("{Enter}");
    const listbox = within(cell(1, STATUS)).getByRole("listbox", {
      name: "Status of Trace sampling knobs",
    });
    expect(document.activeElement).toBe(listbox);
    const options = within(listbox).getAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual([
      "None",
      "Backlog",
      "In progress",
      "In review",
      "Done",
    ]);
    expect(options[4]?.getAttribute("aria-selected")).toBe("true");
    expect(listbox.getAttribute("aria-activedescendant")).toBe(options[4]?.id);
    await user.keyboard("{ArrowUp}{Enter}");
    expect(await within(cell(1, STATUS)).findByText("In review")).toBeDefined();
    expect(document.activeElement).toBe(cell(1, STATUS));
  });

  it("toggles multi-select options and keeps them with Enter", async () => {
    const { user, cell, focus } = await renderTable();
    focus(1, LABELS);
    await user.keyboard("{Enter}");
    const listbox = within(cell(1, LABELS)).getByRole("listbox");
    expect(listbox.getAttribute("aria-multiselectable")).toBe("true");
    await user.keyboard("{Home} ");
    await user.click(within(listbox).getByRole("option", { name: "docs" }));
    expect(
      within(listbox)
        .getByRole("option", { name: "bug" })
        .getAttribute("aria-selected"),
    ).toBe("true");
    await user.keyboard("{Enter}");
    expect(await within(cell(1, LABELS)).findByText("bug")).toBeDefined();
    expect(cell(1, LABELS).textContent).toBe("bug docs performance");
  });

  it("picks a person and clears a relation", async () => {
    const { user, cell, focus } = await renderTable();
    focus(1, OWNER);
    await user.keyboard("{Enter}");
    await user.click(screen.getByRole("option", { name: "Sam Okafor" }));
    expect(await within(cell(1, OWNER)).findByText("Sam Okafor")).toBeDefined();
    focus(1, RELEASE);
    await user.keyboard("{Enter}");
    const listbox = await within(cell(1, RELEASE)).findByRole("listbox");
    expect(
      await within(listbox).findByRole("option", { name: "Gateway beta" }),
    ).toBeDefined();
    await user.click(
      within(listbox).getByRole("option", { name: "Gateway beta" }),
    );
    await user.keyboard("{Escape}");
    expect(cell(1, RELEASE).textContent).toBe("Gateway beta");
    await user.keyboard("{Enter}");
    await user.click(
      await screen.findByRole("option", { name: "Gateway beta" }),
    );
    await user.click(cell(2, 0));
    expect(await within(cell(1, LEFT)).findByText("0")).toBeDefined();
    expect(cell(1, RELEASE).textContent).toBe("");
  });

  it(
    "toggles a checkbox with Space and Enter, and leaves computed cells " +
      "alone",
    async () => {
      const { user, cell, focus } = await renderTable();
      focus(1, BLOCKED);
      await user.keyboard(" ");
      expect(await within(cell(1, BLOCKED)).findByText("Yes")).toBeDefined();
      await user.keyboard("{Enter}");
      expect(await within(cell(1, BLOCKED)).findByText("No")).toBeDefined();
      focus(1, LEFT);
      await user.keyboard("{Enter}");
      await user.dblClick(cell(1, LEFT));
      expect(within(cell(1, LEFT)).queryByRole("textbox")).toBeNull();
    },
  );

  it("shows a refused write inline and clears it after a good one", async () => {
    const { user, cell, focus, collections } = await renderTable();
    const spy = vi
      .spyOn(collections, "updateRow")
      .mockRejectedValueOnce(new Error("conflict: the file changed on disk"));
    focus(1, 0);
    await user.keyboard("{Enter}x{Enter}");
    const alert = await within(cell(1, 0)).findByRole("alert");
    expect(alert.textContent).toBe("conflict: the file changed on disk");
    expect(cell(1, 0).getAttribute("aria-invalid")).toBe("true");
    spy.mockRestore();
    await user.keyboard("{Enter}y{Enter}");
    expect(
      await within(cell(1, 0)).findByText("Trace sampling knobsy"),
    ).toBeDefined();
    expect(within(cell(1, 0)).queryByRole("alert")).toBeNull();
  });

  it("does not write a value that did not change", async () => {
    const { user, focus, collections } = await renderTable();
    const spy = vi.spyOn(collections, "updateRow");
    focus(1, 0);
    await user.keyboard("{Enter}{Enter}");
    expect(spy).not.toHaveBeenCalled();
  });

  it("opens a row with Shift+Enter or its open button", async () => {
    const { user, cell, focus } = await renderTable();
    focus(1, STATUS);
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    const panel = await screen.findByRole("dialog", {
      name: "Trace sampling knobs",
    });
    await user.click(within(panel).getByRole("button", { name: "Close row" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    await user.click(
      within(cell(2, 0)).getByRole("button", { name: /^Open / }),
    );
    expect(await screen.findByRole("dialog")).toBeDefined();
  });

  it("says when nothing matches and when rows were left out", async () => {
    const collections = createMockCollectionsStore();
    const query = collections.query.bind(collections);
    vi.spyOn(collections, "query").mockImplementation(async (input) => {
      const result = await query(input);
      return input.collection_id === COLLECTION.issues
        ? { ...result, total: 20, truncated: true }
        : result;
    });
    renderApp(`/data/${COLLECTION.issues}`, undefined, collections);
    expect(await screen.findByText("Showing 8 of 20 rows.")).toBeDefined();

    const empty = createMockCollectionsStore();
    const original = empty.query.bind(empty);
    vi.spyOn(empty, "query").mockImplementation(async (input) => ({
      ...(await original(input)),
      groups: [],
    }));
    renderApp(`/data/${COLLECTION.issues}`, undefined, empty);
    expect(await screen.findByText("No rows match this view.")).toBeDefined();
  });
});
