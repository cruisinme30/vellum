// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { renderApp } from "../test/render-app";

import { formatDay, monthOf, today } from "./dates";
import { COLLECTION } from "./mock-data";

const dayCell = (label: string) => {
  const cell = screen.getByText(label).closest("td");
  if (!cell) {
    throw new Error(`no day ${label}`);
  }
  return cell;
};

describe("CalendarView", () => {
  it("puts rows on their dates in a month grid", async () => {
    renderApp(`/data/${COLLECTION.issues}?view=Calendar`);
    expect(await screen.findByRole("heading", { level: 2, name: "November 2026" })).toBeDefined();
    const table = screen.getByRole("table", { name: "November 2026" });
    expect(
      within(table)
        .getAllByRole("columnheader")
        .map((h) => h.textContent),
    ).toEqual(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]);
    // November 2026 starts on a Sunday: six weeks, from October 26 to December 6.
    expect(within(table).getAllByRole("row")).toHaveLength(7);
    expect(dayCell("October 26").dataset["outside"]).toBe("true");
    expect(dayCell("November 13").dataset["outside"]).toBeUndefined();
    expect(within(dayCell("November 13")).getByRole("button").textContent).toBe(
      "Token bucket per tenant",
    );
    expect(within(dayCell("November 20")).getAllByRole("button")).toHaveLength(1);
    expect(screen.getByText("One row has no date.")).toBeDefined();
  });

  it("moves between months and back to this one", async () => {
    const { user } = renderApp(`/data/${COLLECTION.issues}?view=Calendar`);
    await screen.findByRole("heading", { level: 2, name: "November 2026" });
    await user.click(screen.getByRole("button", { name: "Next month" }));
    expect(screen.getByRole("heading", { level: 2, name: "December 2026" })).toBeDefined();
    expect(within(dayCell("December 4")).getByRole("button").textContent).toBe(
      "Replay protection for webhooks",
    );
    await user.click(screen.getByRole("button", { name: "Previous month" }));
    await user.click(screen.getByRole("button", { name: "Previous month" }));
    expect(screen.getByRole("heading", { level: 2, name: "October 2026" })).toBeDefined();
    await user.click(screen.getByRole("button", { name: "This month" }));
    const now = formatDay(monthOf(today()), { month: "long", year: "numeric" });
    expect(screen.getByRole("heading", { level: 2, name: now })).toBeDefined();
  });

  it("opens a row from its day", async () => {
    const { user } = renderApp(`/data/${COLLECTION.issues}?view=Calendar`);
    await user.click(await screen.findByRole("button", { name: "Per-route timeouts" }));
    expect(await screen.findByRole("dialog", { name: "Per-route timeouts" })).toBeDefined();
  });
});
