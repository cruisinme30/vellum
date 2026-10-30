// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, waitFor, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { ParseView } from "../api/api.gen";
import { ServerApisProvider } from "../data/server-apis";
import { I18nProvider } from "../i18n/i18n";

import type { TaskView, WorkApi } from "./api";
import {
  QuickAddForm,
  QuickAddProvider,
  usePageTask,
  useQuickAdd,
} from "./quick-add";
import { fakeApi, NONE } from "./test-work";

/**
 * A parse that sets every field, with spans that overlap or run past the text.
 */
function fullParse(text: string): ParseView {
  return {
    title: "Pay rent",
    assignees: ["sam"],
    list: "home",
    fields: {
      start: "2026-11-01",
      when: "today",
      rrule: "FREQ=MONTHLY",
      recur_from: "completion",
      priority: 1,
      labels: ["bills"],
      remind: ["2026-11-01T09:00"],
    },
    highlights: [
      { start: 0, end: 3, kind: "due" },
      // Starts inside the span before it, so it is skipped.
      { start: 2, end: 5, kind: "label" },
      // Runs past the end of the text, so it is dropped.
      { start: 4, end: text.length + 10, kind: "list" },
      // Empty, so it is dropped.
      { start: 4, end: 4, kind: "priority" },
    ],
  };
}

function fakeTask(id: string, title: string): TaskView {
  return {
    id,
    title,
    assignees: [],
    done: false,
    fields: { labels: [], remind: [] },
    kind: "checklist",
    overdue: false,
    subtasks: [],
  };
}

function renderForm(api: WorkApi, onAdded = vi.fn<(title: string) => void>()) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <QuickAddForm api={api} onAdded={onAdded} />
    </I18nProvider>,
  );
  return { user, onAdded };
}

describe("QuickAddForm", () => {
  it("lists everything the parser understood", async () => {
    const { api } = fakeApi({
      parse: (text) => Promise.resolve(fullParse(text)),
    });
    const { user } = renderForm(api);
    await user.type(
      screen.getByRole("textbox", { name: "Add a task" }),
      "Pay rent",
    );
    const facts = await screen.findByRole("list", {
      name: "What Vellum understood",
    });
    const items = within(facts)
      .getAllByRole("listitem")
      .map((item) => item.textContent);
    expect(items).toEqual([
      expect.stringContaining("Starts"),
      "Today",
      expect.stringContaining("Repeats after done"),
      expect.stringContaining("Priority"),
      expect.stringContaining("List"),
      "@bills",
      "+sam",
      expect.stringContaining("Reminder"),
    ]);
    const marks = document.querySelectorAll(".vl-quick-line mark");
    expect([...marks].map((mark) => mark.textContent)).toEqual(["Pay"]);
    expect(document.querySelector(".vl-quick-line")?.textContent).toBe(
      "Pay rent",
    );
  });

  it("says a rule repeats on schedule when it does not count from completion", async () => {
    const { api } = fakeApi({
      parse: (text) => {
        const parsed = fullParse(text);
        return Promise.resolve({
          ...parsed,
          fields: { ...parsed.fields, recur_from: NONE },
        });
      },
    });
    const { user } = renderForm(api);
    await user.type(
      screen.getByRole("textbox", { name: "Add a task" }),
      "Pay rent",
    );
    expect(await screen.findByText(/^Repeats: /)).toBeDefined();
    expect(screen.queryByText(/Repeats after done/)).toBeNull();
  });

  it("shows no facts list when the parse sets nothing", async () => {
    const parse = vi.fn<WorkApi["parse"]>((text) =>
      Promise.resolve({
        title: text,
        assignees: [],
        fields: { labels: [], remind: [] },
        highlights: [{ start: 0, end: 4, kind: "due" }],
      }),
    );
    const { api } = fakeApi({ parse });
    const { user } = renderForm(api);
    await user.type(
      screen.getByRole("textbox", { name: "Add a task" }),
      "soon",
    );
    await waitFor(() => {
      expect(document.querySelector(".vl-quick-line mark")).not.toBeNull();
    });
    expect(
      screen.queryByRole("list", { name: "What Vellum understood" }),
    ).toBeNull();
  });

  it("adds the task without highlights when parsing fails", async () => {
    const parse = vi.fn<WorkApi["parse"]>(() =>
      Promise.reject(new Error("offline")),
    );
    const { api, addTask } = fakeApi({ parse });
    const { user, onAdded } = renderForm(api);
    await user.type(
      screen.getByRole("textbox", { name: "Add a task" }),
      "Call mum",
    );
    await waitFor(() => {
      expect(parse).toHaveBeenCalled();
    });
    expect(document.querySelector(".vl-quick-line")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(addTask).toHaveBeenCalledWith("Call mum", undefined);
    await waitFor(() => {
      expect(onAdded).toHaveBeenCalled();
    });
  });

  it("adds nothing for a blank line", async () => {
    const { api, addTask } = fakeApi();
    const { user } = renderForm(api);
    await user.type(screen.getByRole("textbox", { name: "Add a task" }), "   ");
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(addTask).not.toHaveBeenCalled();
  });

  it("adds to the chosen list", async () => {
    const { api, addTask } = fakeApi();
    const { user } = renderForm(api);
    const list = await screen.findByRole("combobox", { name: "List" });
    await user.selectOptions(list, "list-ops");
    await user.type(
      screen.getByRole("textbox", { name: "Add a task" }),
      "Deploy",
    );
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(addTask).toHaveBeenCalledWith("Deploy", "list-ops");
  });

  it("hides the list picker when lists fail to load", async () => {
    const lists = vi.fn<WorkApi["lists"]>(() =>
      Promise.reject(new Error("down")),
    );
    const { api } = fakeApi({ lists });
    renderForm(api);
    await waitFor(() => {
      expect(lists).toHaveBeenCalled();
    });
    expect(screen.queryByRole("combobox", { name: "List" })).toBeNull();
  });

  it("shows why adding failed and keeps the line", async () => {
    const addTask = vi.fn<WorkApi["addTask"]>(() =>
      Promise.reject(new Error("Quota reached")),
    );
    const { api } = fakeApi({ addTask });
    const { user, onAdded } = renderForm(api);
    const line = screen.getByRole("textbox", { name: "Add a task" });
    await user.type(line, "Write report");
    await user.click(screen.getByRole("button", { name: "Add" }));
    expect(await screen.findByRole("alert")).toBeDefined();
    expect(line).toHaveProperty("value", "Write report");
    expect(onAdded).not.toHaveBeenCalled();
  });

  it("ignores a second Add while the first is saving", async () => {
    const pending: { resolve?: (task: TaskView) => void } = {};
    const addTask = vi.fn<WorkApi["addTask"]>(
      () =>
        new Promise((done) => {
          pending.resolve = done;
        }),
    );
    const { api } = fakeApi({ addTask });
    const { user, onAdded } = renderForm(api);
    await user.type(
      screen.getByRole("textbox", { name: "Add a task" }),
      "Once{Enter}{Enter}",
    );
    expect(addTask).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "Add" })).toHaveProperty(
      "disabled",
      true,
    );
    pending.resolve?.(fakeTask("new", "Once"));
    await waitFor(() => {
      expect(onAdded).toHaveBeenCalledWith("Once");
    });
  });
});

function PageTaskButton({
  page,
}: {
  readonly page?: { id: string; title: string };
}) {
  const task = usePageTask(page);
  return (
    <button
      type="button"
      onClick={() => {
        task?.("Check the numbers");
      }}
    >
      {task === undefined ? "No task" : "Make task"}
    </button>
  );
}

function OpenBlank() {
  const quickAdd = useQuickAdd();
  return (
    <button
      type="button"
      onClick={() => {
        quickAdd?.();
      }}
    >
      Blank
    </button>
  );
}

function renderProvider(work: WorkApi | undefined, children: React.ReactNode) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <ServerApisProvider apis={work === undefined ? {} : { work }}>
        <QuickAddProvider>{children}</QuickAddProvider>
      </ServerApisProvider>
    </I18nProvider>,
  );
  return user;
}

/**
 * Matches a title that ends with a link to page "a b", whose title had no
 * letters.
 */
const STRING_WITH_LINK: unknown = expect.stringContaining("[page](/p/a%20b)");

describe("usePageTask", () => {
  it("is off without a page or without a server", () => {
    const { api } = fakeApi();
    renderProvider(api, <PageTaskButton />);
    expect(screen.getByRole("button", { name: "No task" })).toBeDefined();
  });

  it("is off without a server", () => {
    renderProvider(
      undefined,
      <PageTaskButton page={{ id: "P1", title: "Plan" }} />,
    );
    expect(screen.getByRole("button", { name: "No task" })).toBeDefined();
  });

  it(
    "opens quick-add with the text, linking to a page whose title has only " +
      "brackets",
    async () => {
      const { api, update } = fakeApi();
      const user = renderProvider(
        api,
        <PageTaskButton page={{ id: "a b", title: "[]" }} />,
      );
      await user.click(screen.getByRole("button", { name: "Make task" }));
      const dialog = screen.getByRole("dialog", { name: "Quick-add a task" });
      expect(within(dialog).getByText(/links back to/)).toBeDefined();
      await user.click(within(dialog).getByRole("button", { name: "Add" }));
      await waitFor(() => {
        expect(update).toHaveBeenCalledWith("new", {
          title: STRING_WITH_LINK,
        });
      });
    },
  );
});

describe("the blank quick-add dialog", () => {
  it("starts empty, ignores other keys and closes with Cancel", async () => {
    const { api } = fakeApi();
    const user = renderProvider(api, <OpenBlank />);
    await user.click(screen.getByRole("button", { name: "Blank" }));
    const dialog = screen.getByRole("dialog", { name: "Quick-add a task" });
    expect(within(dialog).queryByText(/links back to/)).toBeNull();
    const line = within(dialog).getByRole("textbox", { name: "Add a task" });
    expect(line).toHaveProperty("value", "");
    await user.keyboard("x");
    expect(screen.getByRole("dialog")).toBeDefined();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(
      screen.getByRole("button", { name: "Blank" }),
    );
  });
});
