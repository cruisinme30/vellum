// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import type { Editor } from "@tiptap/core";
import { describe, expect, it, vi } from "vitest";

import { body, issue, p } from "../content";
import { english, renderEditor, typeText } from "../test-editor";

import { statusCache } from "./issue-chip";
import {
  dateItems,
  isoDate,
  linkItems,
  mentionItems,
  type ChipSources,
} from "./items";

// Thursday 29 October 2026, local time.
const NOW = new Date(2026, 9, 29, 14, 0);

const sources: ChipSources = {
  people: () => [
    { handle: "priya", name: "Priya Shah" },
    { handle: "sam", name: "Sam Okafor" },
  ],
  pages: () => [
    { id: "p1", title: "Roadmap" },
    { id: "p2", title: "Release notes" },
    { id: "p3", title: "" },
  ],
  issues: (query) =>
    Promise.resolve(
      [
        { key: "ENG-142", title: "Release blocker" },
        { key: "ENG-7", title: "Fix sign-in" },
      ].filter((issue) =>
        issue.title.toLowerCase().includes(query.toLowerCase()),
      ),
    ),
};

const press = (editor: Editor, key: string) => {
  act(() => {
    fireEvent.keyDown(editor.view.dom, { key });
  });
};

async function typeInto(text: string, chips: ChipSources = sources) {
  const { editor } = await renderEditor(body(p()), { chips });
  act(() => {
    editor.commands.focus("end");
    typeText(editor, text);
  });
  return editor;
}

describe("chip entries", () => {
  it("offers people by handle or name, then dates", () => {
    const titles = mentionItems("sa", sources, english, NOW).map(
      (item) => item.title,
    );
    expect(titles).toEqual(["Sam Okafor"]);
    const all = mentionItems("", sources, english, NOW).map(
      (item) => item.node,
    );
    expect(all).toEqual(["mention", "mention", "date", "date", "date"]);
  });

  it("works out tomorrow and next Monday, and accepts a typed date", () => {
    const dates = dateItems("", english, NOW).map((item) => item.attrs["date"]);
    expect(dates).toEqual(["today", "2026-10-30", "2026-11-02"]);
    const monday = new Date(2026, 10, 2);
    expect(dateItems("next", english, monday)[0]?.attrs["date"]).toBe(
      "2026-11-09",
    );
    expect(dateItems("2026-12-24", english, NOW)[0]?.attrs["date"]).toBe(
      "2026-12-24",
    );
    expect(dateItems("2026-13-01", english, NOW)).toEqual([]);
    expect(isoDate(new Date(2026, 0, 5))).toBe("2026-01-05");
  });

  it("offers pages, then issues, and keeps pages when Work fails", async () => {
    const found = await linkItems("r", sources, english);
    expect(found.map((item) => [item.node, item.title])).toEqual([
      ["wikiLink", "Roadmap"],
      ["wikiLink", "Release notes"],
      ["issueKey", "ENG-142"],
    ]);
    const failing = {
      ...sources,
      issues: () => Promise.reject(new Error("down")),
    };
    const kept = await linkItems("road", failing, english);
    expect(kept.map((item) => item.title)).toEqual(["Roadmap"]);
    // An empty query lists pages only, so the menu never asks Work for
    // everything.
    const issues = vi.fn(sources.issues);
    await linkItems("", { ...sources, issues }, english);
    expect(issues).not.toHaveBeenCalled();
  });
});

describe("chip menus", () => {
  it("inserts a mention from the @ menu", async () => {
    const editor = await typeInto("Ask @pri");
    const list = await screen.findByRole("listbox", {
      name: "People and dates",
    });
    expect(list.textContent).toContain("Priya Shah");
    press(editor, "Enter");
    const mention = editor.view.dom.querySelector(".vl-mention");
    expect(mention?.textContent).toBe("@priya");
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("inserts a date chip from the @ menu", async () => {
    const editor = await typeInto("Due @tomor");
    await screen.findByRole("listbox", { name: "People and dates" });
    press(editor, "Enter");
    const date = editor.view.dom.querySelector("time.vl-date");
    expect(date?.getAttribute("datetime")).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });

  it("links a page or an issue from the [[ menu", async () => {
    const editor = await typeInto("See [[Release");
    const list = await screen.findByRole("listbox", {
      name: "Pages and issues",
    });
    await waitFor(() => {
      expect(list.textContent).toContain("ENG-142");
    });
    press(editor, "Enter");
    expect(
      editor.view.dom.querySelector(".vl-wikilink")?.textContent,
    ).toContain("Release notes");

    act(() => {
      typeText(editor, "and [[Release");
    });
    await screen.findByRole("listbox", { name: "Pages and issues" });
    await waitFor(() => {
      expect(screen.getAllByRole("option")).toHaveLength(2);
    });
    press(editor, "ArrowDown");
    press(editor, "Enter");
    expect(editor.view.dom.querySelector(".vl-issue")?.textContent).toBe(
      "ENG-142",
    );
  });

  it("has no menus without sources", async () => {
    const { editor } = await renderEditor(body(p()));
    act(() => {
      editor.commands.focus("end");
      typeText(editor, "@pri");
    });
    expect(screen.queryByRole("listbox")).toBeNull();
  });
});

describe("live issue chips", () => {
  it("shows the state and title, and opens the issue on click", async () => {
    const openIssue = vi.fn<(key: string) => void>();
    const { editor } = await renderEditor(body(p(issue("ENG-7"))), {
      chips: {
        ...sources,
        issueStatus: () =>
          Promise.resolve({
            title: "Fix sign-in",
            state: "Done",
            closed: true,
          }),
        openIssue,
      },
    });
    const chip = editor.view.dom.querySelector<HTMLElement>(".vl-issue");
    await waitFor(() => {
      expect(chip?.textContent).toBe("ENG-7Done");
    });
    expect(chip?.dataset["closed"]).toBe("true");
    expect(chip?.getAttribute("aria-label")).toBe("ENG-7: Fix sign-in (Done)");
    expect(editor.getText()).toContain("ENG-7");
    chip?.click();
    expect(openIssue).toHaveBeenCalledWith("ENG-7");
  });

  it("asks Work once per issue a minute, and forgets failures", async () => {
    let time = 0;
    const lookup = vi.fn((key: string) =>
      key === "BAD-1"
        ? Promise.reject(new Error("gone"))
        : Promise.resolve({ title: key, state: "Todo", closed: false }),
    );
    const cached = statusCache(lookup, () => time);
    await cached("ENG-1");
    await cached("ENG-1");
    expect(lookup).toHaveBeenCalledTimes(1);
    time = 61_000;
    await cached("ENG-1");
    expect(lookup).toHaveBeenCalledTimes(2);
    expect(await cached("BAD-1")).toBeUndefined();
  });
});
