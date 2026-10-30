// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The small pieces under the views: context, values, dates and the picker.
import { renderHook, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { renderWithProviders } from "../test/render-app";

import { useCollections } from "./context";
import {
  addDays,
  addMonths,
  daysBetween,
  monthOf,
  monthWeeks,
  today,
} from "./dates";
import { choiceValue, groupValue, propertyOf } from "./editing";
import { OptionPicker } from "./option-picker";
import type { Value } from "./types";
import {
  bool,
  date,
  dayOf,
  EMPTY,
  inputText,
  isEmpty,
  list,
  number,
  parseInput,
  plain,
  sameValue,
  text,
  texts,
} from "./value";

describe("useCollections", () => {
  it("needs a provider", () => {
    vi.spyOn(console, "error").mockImplementation(() => {
      // React reports the thrown error; the test expects it.
    });
    expect(() => renderHook(() => useCollections())).toThrow(
      /needs a CollectionsProvider/,
    );
  });
});

/** A value that is not there, as an empty cell's is. */
let missing: Value | undefined;

describe("values", () => {
  it("knows empty values and their texts", () => {
    expect(isEmpty(missing)).toBe(true);
    expect(isEmpty(EMPTY)).toBe(true);
    expect(isEmpty(text(""))).toBe(true);
    expect(isEmpty(list([EMPTY, text("")]))).toBe(true);
    expect(isEmpty(number(0))).toBe(false);
    expect(texts(list([text("a"), list([text("b")]), number(1)]))).toEqual([
      "a",
      "b",
    ]);
    expect(texts(text(""))).toEqual([]);
  });

  it("formats values as plain text", () => {
    expect(plain(number(0.1 + 0.2))).toBe("0.3");
    expect(plain(number(5))).toBe("5");
    expect(plain(bool(true))).toBe("Yes");
    expect(plain(bool(false))).toBe("No");
    expect(plain(date("2026-11-02T09:30"))).toBe("2026-11-02T09:30");
    expect(plain(list([text("a"), EMPTY, number(2)]))).toBe("a, 2");
    expect(plain(EMPTY)).toBe("");
    expect(dayOf(date("2026-11-02T09:30"))).toBe("2026-11-02");
    expect(dayOf(text("2026-11-02"))).toBe("");
    expect(inputText(number(2.5))).toBe("2.5");
    expect(inputText(date("2026-11-02T09:30"))).toBe("2026-11-02");
    expect(inputText(missing)).toBe("");
  });

  it("parses typed input by property type", () => {
    expect(parseInput("title", " ")).toEqual({ value: text("") });
    expect(parseInput("text", "")).toEqual({ value: EMPTY });
    expect(parseInput("number", "1,500.5")).toEqual({ value: number(1500.5) });
    expect(parseInput("number", "1e400")).toEqual({
      error: "collection-error-number",
    });
    expect(parseInput("date", "2026-11-02")).toEqual({
      value: date("2026-11-02"),
    });
    expect(parseInput("date", "2026-13-45")).toEqual({
      error: "collection-error-date",
    });
    expect(parseInput("date", "soon")).toEqual({
      error: "collection-error-date",
    });
    expect(parseInput("url", "https://x.example")).toEqual({
      value: text("https://x.example"),
    });
    expect(parseInput("url", "x")).toEqual({ error: "collection-error-url" });
    expect(sameValue(undefined, EMPTY)).toBe(true);
    expect(sameValue(text("a"), text("b"))).toBe(false);
  });

  it("turns picks and groups into values", () => {
    expect(choiceValue("select", [])).toEqual(EMPTY);
    expect(choiceValue("select", ["Done"])).toEqual(text("Done"));
    expect(choiceValue("multi", ["a", "b"])).toEqual(
      list([text("a"), text("b")]),
    );
    // eslint-disable-next-line unicorn/no-null -- the "No …" group's value
    expect(groupValue("select", null)).toEqual(EMPTY);
    expect(groupValue("relation", text("r"))).toEqual(list([text("r")]));
    expect(groupValue("checkbox", bool(true))).toEqual(bool(true));
    expect(
      propertyOf([], {
        key: "x",
        name: "X",
        type: "select",
        format: "plain",
        editable: false,
      }),
    ).toEqual({ key: "x", name: "X", type: "select", computed: true });
    expect(
      propertyOf([], {
        key: "x",
        name: "X",
        type: "select",
        format: "plain",
        editable: true,
        options: [{ name: "A" }],
      }).options,
    ).toEqual([{ name: "A" }]);
  });
});

describe("dates", () => {
  it("does calendar arithmetic in UTC", () => {
    expect(addDays("2026-10-31", 1)).toBe("2026-11-01");
    expect(daysBetween("2026-10-26", "2026-11-03")).toBe(8);
    expect(monthOf("2026-11-13")).toBe("2026-11-01");
    expect(addMonths("2026-12-01", 1)).toBe("2027-01-01");
    expect(today(new Date("2026-10-29T23:30:00Z"))).toBe("2026-10-29");
    const weeks = monthWeeks("2026-02-01");
    expect(weeks[0]?.[0]).toBe("2026-01-26");
    expect(weeks.at(-1)?.at(-1)).toBe("2026-03-01");
    expect(monthWeeks("2027-02-01")).toHaveLength(4);
  });
});

describe("OptionPicker", () => {
  const choices = [
    { id: "a", label: "Alpha" },
    { id: "b", label: "Beta", color: "blue" },
  ];

  function renderPicker(multiple: boolean, selected: string[] = []) {
    const onCommit = vi.fn<(ids: string[]) => void>();
    const onCancel = vi.fn<() => void>();
    renderWithProviders(
      <div>
        <button type="button">Elsewhere</button>
        <OptionPicker
          label="Pick"
          choices={choices}
          selected={selected}
          multiple={multiple}
          onCommit={onCommit}
          onCancel={onCancel}
        />
      </div>,
    );
    return {
      onCommit,
      onCancel,
      listbox: screen.getByRole("listbox", { name: "Pick" }),
    };
  }

  it("picks one with the keyboard, or none", async () => {
    const user = userEvent.setup();
    const { onCommit, listbox } = renderPicker(false, ["b"]);
    expect(document.activeElement).toBe(listbox);
    expect(
      within(listbox).getByRole("option", { name: "Beta" }).dataset["active"],
    ).toBe("true");
    await user.keyboard("{End}{ArrowDown}{Home}{Enter}");
    expect(onCommit).toHaveBeenLastCalledWith([]);
  });

  it("picks one with Space", async () => {
    const user = userEvent.setup();
    const { onCommit, onCancel } = renderPicker(false);
    await user.keyboard("{ArrowDown} ");
    expect(onCommit).toHaveBeenLastCalledWith(["a"]);
    expect(onCancel).not.toHaveBeenCalled();
  });

  it("cancels a single pick when focus leaves", async () => {
    const user = userEvent.setup();
    const { onCommit, onCancel } = renderPicker(false);
    await user.click(screen.getByRole("button", { name: "Elsewhere" }));
    expect(onCancel).toHaveBeenCalledOnce();
    expect(onCommit).not.toHaveBeenCalled();
  });

  it("keeps several picks in choice order when focus leaves", async () => {
    const user = userEvent.setup();
    const { onCommit, listbox } = renderPicker(true, ["b"]);
    await user.click(within(listbox).getByRole("option", { name: "Alpha" }));
    await user.keyboard("{ArrowDown} ");
    await user.click(screen.getByRole("button", { name: "Elsewhere" }));
    expect(onCommit).toHaveBeenCalledExactlyOnceWith(["a"]);
  });

  it("says when there is nothing to pick", () => {
    const onCommit = vi.fn<(ids: string[]) => void>();
    renderWithProviders(
      <OptionPicker
        label="Empty"
        choices={[]}
        selected={[]}
        multiple
        onCommit={onCommit}
        onCancel={() => {
          onCommit([]);
        }}
      />,
    );
    expect(screen.getByText("Nothing to choose from.")).toBeDefined();
    expect(
      screen.getByRole("listbox").hasAttribute("aria-activedescendant"),
    ).toBe(false);
  });
});
