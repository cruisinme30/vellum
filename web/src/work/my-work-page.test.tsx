// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import { Route } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { MyWorkView, TaskView, WorkApi } from "./api";
import { MyWorkPage } from "./my-work-page";
import { fakeApi, renderAt } from "./test-work";

function task(
  id: string,
  title: string,
  extra: Partial<TaskView> = {},
): TaskView {
  return {
    id,
    title,
    assignees: [],
    done: false,
    fields: { labels: [], remind: [] },
    kind: "checklist",
    overdue: false,
    subtasks: [],
    ...extra,
  };
}

const VIEW: MyWorkView = {
  open: 3,
  today: "2026-10-29",
  sections: [
    {
      bucket: "inbox",
      tasks: [
        task("mention.p1", "see the launch notes", {
          kind: "mention",
          doc: "Launch Plan",
        }),
        task("mention.p2", "a stray mention", { kind: "mention" }),
      ],
    },
    {
      bucket: "upcoming",
      day: "2026-11-02",
      tasks: [task("D2.b1", "Renew passport", { overdue: true, list: "home" })],
    },
    {
      bucket: "done",
      tasks: [task("D3.b1", "Paid rent", { done: true })],
    },
  ],
};

const TASK_DRAG = "application/x-vellum-task";

function transfer(types: string[], data: string) {
  return {
    types,
    getData: () => data,
    setData: vi.fn(),
    dropEffect: "none",
    effectAllowed: "all",
  };
}

/**
 * The nearest `selector` around `element`; fails the test when there is none.
 */
function around(element: Element, selector: string): HTMLElement {
  const found = element.closest<HTMLElement>(selector);
  if (found === null) throw new Error(`no ${selector}`);
  return found;
}

/** The section a heading names. */
function sectionOf(name: string): HTMLElement {
  return around(screen.getByRole("heading", { name }), "section");
}

function renderPage(overrides: Partial<WorkApi> = {}) {
  const fake = fakeApi({ myWork: () => Promise.resolve(VIEW), ...overrides });
  const user = renderAt(
    "/my-work",
    fake.api,
    <Route path="my-work" element={<MyWorkPage />} />,
  );
  return { ...fake, user };
}

describe("My Work rows", () => {
  it(
    "links a page mention to its page and shows a mention with no source " +
      "plainly",
    async () => {
      renderPage();
      const link = await screen.findByRole("link", { name: "Open page" });
      expect(link.getAttribute("href")).toBe("/p/Launch%20Plan");
      expect(
        screen.getByText("a stray mention").closest("li")?.querySelector("a"),
      ).toBeNull();
    },
  );

  it("marks overdue tasks, names their list and heads a day by its date", async () => {
    renderPage();
    const row = around(await screen.findByText("Renew passport"), "li");
    expect(row.className).toContain("vl-task-overdue");
    expect(within(row).getByText("#home")).toBeDefined();
    expect(screen.getByRole("heading", { name: "2026-11-02" })).toBeDefined();
  });

  it("does not drag or move a finished task", async () => {
    renderPage();
    const row = around(await screen.findByText("Paid rent"), "li");
    expect(row.getAttribute("draggable")).toBe("false");
    expect(
      screen.queryByRole("combobox", { name: "Move “Paid rent”" }),
    ).toBeNull();
    fireEvent.dragStart(row, { dataTransfer: transfer([TASK_DRAG], "D3.b1") });
    expect(
      screen.queryByRole("list", { name: "Drop the task on a day or bucket" }),
    ).toBeNull();
  });

  it("ignores the Move menu's placeholder", async () => {
    const { move } = renderPage();
    const menu = await screen.findByRole("combobox", {
      name: "Move “Renew passport”",
    });
    fireEvent.change(menu, { target: { value: "" } });
    expect(move).not.toHaveBeenCalled();
  });

  it("ignores drags that carry no task, and tasks it does not know", async () => {
    const { move } = renderPage();
    await screen.findByRole("heading", { name: "2026-11-02" });
    const section = sectionOf("2026-11-02");
    const other = transfer(["text/plain"], "");
    expect(fireEvent.dragOver(section, { dataTransfer: other })).toBe(true);
    fireEvent.drop(section, { dataTransfer: other });
    fireEvent.drop(section, { dataTransfer: transfer([TASK_DRAG], "nope") });
    expect(move).not.toHaveBeenCalled();
    // Done completes tasks rather than taking drops.
    const done = sectionOf("Done");
    fireEvent.drop(done, { dataTransfer: transfer([TASK_DRAG], "D2.b1") });
    expect(move).not.toHaveBeenCalled();
  });

  it("moves a dropped task onto an Upcoming day", async () => {
    const { move } = renderPage();
    const row = around(await screen.findByText("Renew passport"), "li");
    const data = transfer([TASK_DRAG], "D2.b1");
    fireEvent.dragStart(row, { dataTransfer: data });
    const section = sectionOf("2026-11-02");
    expect(section.className).toBe("vl-drop-target");
    fireEvent.dragEnd(row);
    fireEvent.drop(section, { dataTransfer: data });
    expect(move).toHaveBeenCalledWith("D2.b1", "2026-11-02");
  });

  it("shows why a move failed", async () => {
    const move = vi.fn<WorkApi["move"]>(() =>
      Promise.reject(new Error("Locked")),
    );
    renderPage({ move });
    const menu = await screen.findByRole("combobox", {
      name: "Move “Renew passport”",
    });
    fireEvent.change(menu, { target: { value: "someday" } });
    await waitFor(() => {
      expect(screen.getByRole("alert").textContent).toContain("Locked");
    });
  });

  it("says when there is nothing to do", async () => {
    renderPage({
      myWork: () =>
        Promise.resolve({ open: 0, today: "2026-10-29", sections: [] }),
    });
    expect(await screen.findByText(/nothing/i)).toBeDefined();
  });
});
