// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/* eslint-disable @typescript-eslint/unbound-method --
   the API's methods are vi.fn mocks */

import { screen, within } from "@testing-library/react";
import { Route } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { CYCLE, fakeApi, issue, renderAt, TEAM } from "../test/work-fixtures";

import type { CycleOut } from "./api";
import { CyclesPage, cycleState } from "./cycles-page";
import { MyIssuesPage } from "./my-issues-page";
import { snoozeUntil, TriagePage } from "./triage-page";

describe("triage", () => {
  const waiting = [
    issue("ENG-9", "Login loop", {
      state: "triage",
      category: "triage",
      creator: "sam",
      sla: {
        kind: "triage",
        breaches_at: "2026-10-30T09:00:00Z",
        risk: "orange",
      },
    }),
  ];

  it("accepts, declines, duplicates and snoozes", async () => {
    const api = fakeApi({ inbox: vi.fn(() => Promise.resolve(waiting)) });
    const user = renderAt(
      "/issues/triage",
      api,
      <Route path="issues/triage" element={<TriagePage />} />,
    );
    const row = await screen.findByRole("link", { name: /ENG-9/ });
    expect(row.textContent).toContain("SLA at risk");
    expect(screen.getByText("Filed by @sam")).toBeDefined();
    expect(
      within(screen.getByRole("combobox", { name: "Accept into" })).queryByText(
        "Done",
      ),
    ).toBeNull();

    await user.selectOptions(
      screen.getByRole("combobox", { name: "Accept into" }),
      "doing",
    );
    await user.click(screen.getByRole("button", { name: "Accept" }));
    expect(api.triage).toHaveBeenCalledWith("ENG-9", {
      action: "accept",
      state: "doing",
    });

    await user.click(await screen.findByRole("button", { name: "Decline" }));
    await user.type(
      screen.getByRole("textbox", { name: "Reason (optional)" }),
      "Won't fix",
    );
    await user.click(screen.getByRole("button", { name: "Decline issue" }));
    expect(api.triage).toHaveBeenCalledWith("ENG-9", {
      action: "decline",
      reason: "Won't fix",
    });

    await user.click(await screen.findByRole("button", { name: "Duplicate" }));
    await user.type(
      screen.getByRole("textbox", { name: "Duplicate of" }),
      "eng-3",
    );
    await user.click(screen.getByRole("button", { name: "Mark as duplicate" }));
    expect(api.triage).toHaveBeenCalledWith("ENG-9", {
      action: "duplicate",
      duplicate_of: "ENG-3",
    });

    await user.click(await screen.findByRole("button", { name: "Snooze" }));
    await user.selectOptions(
      screen.getByRole("combobox", { name: "Snooze for" }),
      "7",
    );
    await user.click(screen.getByRole("button", { name: "Snooze issue" }));
    expect(api.triage).toHaveBeenLastCalledWith(
      "ENG-9",
      expect.objectContaining({
        action: "snooze",
        until: expect.stringMatching(/Z$/) as unknown,
      }),
    );
    expect(snoozeUntil(1, new Date("2026-10-29T00:00:00Z"))).toBe(
      "2026-10-30T00:00:00.000Z",
    );
  });

  it("says when a team has no triage", async () => {
    const api = fakeApi({
      teams: () => Promise.resolve([{ ...TEAM, triage: false }]),
    });
    renderAt(
      "/issues/triage",
      api,
      <Route path="issues/triage" element={<TriagePage />} />,
    );
    expect(await screen.findByText(/doesn't use triage/)).toBeDefined();
  });
});

describe("cycles", () => {
  it("shows the current cycle's burn-up and the velocity", async () => {
    const past: CycleOut = {
      ...CYCLE,
      key: "2026-C21",
      starts_on: "2026-10-12",
      ends_on: "2026-10-26",
      burn_up: [],
    };
    const api = fakeApi({
      cycles: vi.fn(() =>
        Promise.resolve({ cycles: [past, CYCLE], velocity: 7.25 }),
      ),
    });
    renderAt(
      "/issues/cycles",
      api,
      <Route path="issues/cycles" element={<CyclesPage on="2026-10-29" />} />,
    );
    expect(
      await screen.findByRole("heading", { name: "Current: 2026-C22" }),
    ).toBeDefined();
    expect(screen.getByText(/Velocity: 7.3 points/)).toBeDefined();
    expect(
      screen.getByRole("img", {
        name: "Burn-up for 2026-C22: 5 of 12 points completed.",
      }),
    ).toBeDefined();
    const table = screen.getAllByRole("table").at(-1);
    expect(table?.textContent).toContain("Finished");
    expect(table?.textContent).toContain("Running");
    const link = screen.getByRole("link", { name: "Open on the board" });
    expect(decodeURIComponent(link.getAttribute("href") ?? "")).toContain(
      '"field":"cycle","op":"is","values":["2026-C22"]',
    );
    expect(cycleState(CYCLE, "2026-11-09")).toBe("past");
    expect(cycleState(CYCLE, "2026-10-01")).toBe("upcoming");
  });
});

describe("my issues", () => {
  it("groups assigned issues in focus order and switches tabs", async () => {
    const myIssues = vi.fn((tab: string) =>
      Promise.resolve({
        issues:
          tab === "assigned"
            ? [
                { focus: "urgent", issue: issue("ENG-1", "Fire") },
                { focus: "sla", issue: issue("ENG-2", "Clock") },
                { focus: "sla", issue: issue("ENG-3", "Clock 2") },
              ]
            : [],
      }),
    );
    const user = renderAt(
      "/issues/mine",
      fakeApi({ myIssues }),
      <Route path="issues/mine" element={<MyIssuesPage />} />,
    );
    expect(
      await screen.findByRole("heading", { name: "Urgent 1" }),
    ).toBeDefined();
    expect(screen.getByRole("heading", { name: "SLA-bound 2" })).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Subscribed" }));
    expect(
      await screen.findByText("You aren't subscribed to any issues."),
    ).toBeDefined();
    expect(myIssues).toHaveBeenLastCalledWith("subscribed");
  });
});
