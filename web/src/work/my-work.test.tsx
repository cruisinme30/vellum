// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { Route } from "react-router";
import { describe, expect, it, vi } from "vitest";

import { ServerApisProvider } from "../data/server-apis";
import { I18nProvider } from "../i18n/i18n";

import { MyWorkPage } from "./my-work-page";
import { QuickAddProvider, useQuickAdd } from "./quick-add";
import { fakeApi, MENTIONED, NONE, renderAt } from "./test-work";

describe("My Work", () => {
  it("lists tasks by bucket, ticks them and adds new ones", async () => {
    const { api, myWork, addTask, setDone } = fakeApi();
    const user = renderAt("/my-work", api, <Route path="my-work" element={<MyWorkPage />} />);
    expect(await screen.findByRole("heading", { name: "Today" })).toBeDefined();
    expect(screen.queryByRole("heading", { name: "Inbox" })).toBeNull();
    expect(screen.getByText("Due 2026-10-29")).toBeDefined();
    expect(screen.getByRole("link", { name: "ENG-7" }).getAttribute("href")).toBe("/issues/ENG-7");

    await user.click(screen.getByRole("checkbox", { name: "Write the launch post" }));
    expect(setDone).toHaveBeenCalledWith("k1", true);

    await user.type(screen.getByRole("textbox", { name: "Add a task" }), "Call Sam tomorrow");
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(addTask).toHaveBeenCalledWith("Call Sam tomorrow", undefined);
    await waitFor(() => {
      expect(myWork).toHaveBeenCalledTimes(3);
    });
  });

  it("holds the panels back until the buckets answer, so they do not jump down", async () => {
    const answers: ((view: typeof MENTIONED) => void)[] = [];
    const { api } = fakeApi({
      myWork: () =>
        new Promise((resolve) => {
          answers.push(resolve);
        }),
    });
    renderAt("/my-work", api, <Route path="my-work" element={<MyWorkPage />} />);
    expect(await screen.findByRole("status")).toBeDefined();
    expect(screen.queryByText("Lists")).toBeNull();
    for (const answer of answers) answer(MENTIONED);
    expect(await screen.findByText("Lists")).toBeDefined();
    expect(screen.getByText("Google Calendar")).toBeDefined();
  });

  it("shows who mentioned you and dismisses the mention", async () => {
    const { api, dismissMention } = fakeApi({ myWork: () => Promise.resolve(MENTIONED) });
    const user = renderAt("/my-work", api, <Route path="my-work" element={<MyWorkPage />} />);
    expect(await screen.findByText("priya mentioned you:")).toBeDefined();
    expect(screen.queryByRole("checkbox")).toBeNull();
    expect(screen.getByRole("link", { name: "ENG-7" }).getAttribute("href")).toBe("/issues/ENG-7");
    await user.click(
      screen.getByRole("button", { name: "Dismiss the mention “@ana can you check the copy?”" }),
    );
    expect(dismissMention).toHaveBeenCalledWith("mention.c9");
  });

  it("saves the time zone, start of day and digest time", async () => {
    const { api, saveSettings } = fakeApi();
    const user = renderAt("/my-work", api, <Route path="my-work" element={<MyWorkPage />} />);
    await user.click(await screen.findByText("Time zone and reminders"));
    expect(await screen.findByText("My Work counts days in UTC.")).toBeDefined();
    const zone = screen.getByRole("combobox", { name: "Time zone" });
    await user.clear(zone);
    await user.type(zone, "Asia/Tokyo");
    await user.clear(screen.getByLabelText("Morning digest at"));
    await user.click(screen.getByRole("button", { name: "Save settings" }));
    expect(saveSettings).toHaveBeenCalledWith({
      time_zone: "Asia/Tokyo",
      day_starts_at: "09:00",
      digest_at: NONE,
    });
    expect(await screen.findByText("Settings saved.")).toBeDefined();
  });

  it("highlights what quick-add understood and adds to a list", async () => {
    const { api, addTask } = fakeApi();
    const user = renderAt("/my-work", api, <Route path="my-work" element={<MyWorkPage />} />);
    const line = await screen.findByRole("textbox", { name: "Add a task" });
    await user.type(line, "Pay rent tomorrow 3pm #ops");
    const due = await screen.findByText("tomorrow 3pm");
    expect(due.tagName).toBe("MARK");
    expect(due.dataset["kind"]).toBe("due");
    const facts = screen.getByRole("list", { name: "What Vellum understood" });
    expect(within(facts).getByText("Due 2026-10-30T15:00")).toBeDefined();
    expect(within(facts).getByText("List ops")).toBeDefined();
    await user.selectOptions(screen.getByRole("combobox", { name: "List" }), "list-ops");
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(addTask).toHaveBeenCalledWith("Pay rent tomorrow 3pm #ops", "list-ops");
  });

  it("reschedules from the Move menu and by dragging onto a section", async () => {
    const { api, move } = fakeApi();
    const user = renderAt("/my-work", api, <Route path="my-work" element={<MyWorkPage />} />);
    const menu = await screen.findByRole("combobox", { name: "Move “Book flights”" });
    // Issues and tasks without a block ID cannot move.
    expect(screen.queryByRole("combobox", { name: "Move “Fix sign-in”" })).toBeNull();
    await user.selectOptions(menu, "someday");
    expect(move).toHaveBeenCalledWith("D1.b2", "someday");

    const row = screen.getByText("Book flights").closest("li");
    expect(row?.getAttribute("draggable")).toBe("true");
    const dataTransfer = {
      types: ["application/x-vellum-task"],
      getData: () => "D1.b2",
      setData: vi.fn(),
      dropEffect: "none",
      effectAllowed: "all",
    };
    fireEvent.dragStart(row as HTMLElement, { dataTransfer });
    const bar = screen.getByRole("list", { name: "Drop the task on a day or bucket" });
    fireEvent.dragOver(within(bar).getByText("Tomorrow"), { dataTransfer });
    fireEvent.drop(within(bar).getByText("Tomorrow"), { dataTransfer });
    expect(move).toHaveBeenLastCalledWith("D1.b2", "2026-10-30");

    const today = screen.getByRole("heading", { name: "Today" }).parentElement;
    if (today === null) throw new Error("no Today section");
    fireEvent.drop(today, { dataTransfer });
    expect(move).toHaveBeenLastCalledWith("D1.b2", "today");
  });

  it("shows and changes a task's time block", async () => {
    const { api, update } = fakeApi();
    const user = renderAt("/my-work", api, <Route path="my-work" element={<MyWorkPage />} />);
    expect(await screen.findByText("Blocked 15:00–16:30, 2026-10-29")).toBeDefined();
    await user.click(screen.getByRole("button", { name: "Block time for “Book flights”" }));
    const form = screen.getByRole("form", { name: "Block time for “Book flights”" });
    const minutes = within(form).getByRole("spinbutton", { name: "Minutes" });
    await user.clear(minutes);
    await user.type(minutes, "30");
    await user.click(within(form).getByRole("button", { name: "Save block" }));
    expect(update).toHaveBeenCalledWith("D1.b2", { block: "2026-10-29T15:00/30" });
  });

  it("creates and shares lists, and leaves others' lists alone", async () => {
    const { api, shareList, createList } = fakeApi();
    const user = renderAt("/my-work", api, <Route path="my-work" element={<MyWorkPage />} />);
    await user.click(await screen.findByText("Lists"));
    expect(await screen.findByText("shared by @priya", { exact: false })).toBeDefined();
    expect(screen.queryByRole("textbox", { name: "Share Home with" })).toBeNull();
    await user.type(screen.getByRole("textbox", { name: "Share Ops with" }), "@sam");
    await user.click(screen.getByRole("button", { name: "Share" }));
    expect(shareList).toHaveBeenCalledWith("list-ops", "@sam", true);
    await user.type(screen.getByRole("textbox", { name: "New list" }), "Groceries");
    await user.click(screen.getByRole("button", { name: "Create list" }));
    expect(createList).toHaveBeenCalledWith("Groceries");
  });

  it("syncs Google Calendar time blocks", async () => {
    const { api, syncGoogle } = fakeApi();
    const user = renderAt("/my-work", api, <Route path="my-work" element={<MyWorkPage />} />);
    await user.click(await screen.findByText("Google Calendar"));
    await user.click(await screen.findByRole("button", { name: "Sync now" }));
    expect(syncGoogle).toHaveBeenCalledOnce();
    expect(await screen.findByText("Synced 3 changes.")).toBeDefined();
  });

  it("explains itself in demo mode", () => {
    renderAt("/my-work", undefined, <Route path="my-work" element={<MyWorkPage />} />);
    expect(screen.getByText(/needs a Vellum server/)).toBeDefined();
  });
});

function OpenFromPage() {
  const quickAdd = useQuickAdd();
  return (
    <button
      type="button"
      onClick={() => {
        quickAdd?.({ text: "Review the plan", page: { id: "P1", title: "Launch [draft]" } });
      }}
    >
      From page
    </button>
  );
}

describe("the quick-add dialog", () => {
  it("opens with a page's selection and links the task back to the page", async () => {
    const { api, addTask, update } = fakeApi();
    const user = userEvent.setup();
    render(
      <I18nProvider>
        <ServerApisProvider apis={{ work: api }}>
          <QuickAddProvider>
            <OpenFromPage />
          </QuickAddProvider>
        </ServerApisProvider>
      </I18nProvider>,
    );
    await user.click(screen.getByRole("button", { name: "From page" }));
    const dialog = screen.getByRole("dialog", { name: "Quick-add a task" });
    const line = within(dialog).getByRole("textbox", { name: "Add a task" });
    expect(document.activeElement).toBe(line);
    expect(line).toHaveProperty("value", "Review the plan");
    await user.click(within(dialog).getByRole("button", { name: "Add" }));
    expect(addTask).toHaveBeenCalledWith("Review the plan", undefined);
    await waitFor(() => {
      expect(update).toHaveBeenCalledWith("new", {
        title: "Review the plan [Launch draft](/p/P1)",
      });
    });
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).toBeNull();
    });
  });

  it("closes on Escape and does nothing without a server", async () => {
    const { api } = fakeApi();
    const user = userEvent.setup();
    const { unmount } = render(
      <I18nProvider>
        <ServerApisProvider apis={{ work: api }}>
          <QuickAddProvider>
            <OpenFromPage />
          </QuickAddProvider>
        </ServerApisProvider>
      </I18nProvider>,
    );
    await user.click(screen.getByRole("button", { name: "From page" }));
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    unmount();
    render(
      <I18nProvider>
        <ServerApisProvider apis={{}}>
          <QuickAddProvider>
            <OpenFromPage />
          </QuickAddProvider>
        </ServerApisProvider>
      </I18nProvider>,
    );
    await user.click(screen.getByRole("button", { name: "From page" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
