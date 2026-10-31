// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { createRef } from "react";
import { describe, expect, it, vi } from "vitest";

import { english } from "../test-editor";

import { filterSlashItems, SLASH_ITEMS, type SlashItem } from "./items";
import { SlashMenu, type SlashMenuHandle } from "./slash-menu";

const LIST_ID = "slash-test";

function setup(items: readonly SlashItem[] = filterSlashItems("callout")) {
  const command = vi.fn<(item: SlashItem) => void>();
  const onActive = vi.fn<(id: string | undefined) => void>();
  const handle = createRef<SlashMenuHandle>();
  const result = render(
    <SlashMenu
      items={items}
      command={command}
      t={english}
      listId={LIST_ID}
      onActive={onActive}
      ref={handle}
    />,
  );
  const press = (key: string) => {
    let handled = false;
    act(() => {
      handled =
        handle.current?.onKeyDown(new KeyboardEvent("keydown", { key })) ??
        false;
    });
    return handled;
  };
  return { ...result, command, onActive, press };
}

const selected = () =>
  screen
    .getAllByRole("option")
    .find((option) => option.getAttribute("aria-selected") === "true")
    ?.textContent;

describe("SlashMenu", () => {
  it(
    "lists the matching blocks in a labelled listbox, first one " + "selected",
    () => {
      setup();
      const list = screen.getByRole("listbox", { name: "Blocks" });
      expect(list.id).toBe(LIST_ID);
      expect(within(list).getAllByRole("option")).toHaveLength(6);
      expect(selected()).toContain("Info callout");
    },
  );

  it("groups consecutive entries of a section under its heading", () => {
    setup(SLASH_ITEMS);
    const groups = screen.getAllByRole("group");
    expect(
      groups.map((group) => group.getAttribute("aria-labelledby")),
    ).toContain(`${LIST_ID}-group-0`);
    expect(screen.getByRole("group", { name: "Basic" })).toBeDefined();
    expect(screen.getByRole("group", { name: "Callouts" })).toBeDefined();
  });

  it("reports the highlighted option id for aria-activedescendant", () => {
    const { onActive, press } = setup();
    expect(onActive).toHaveBeenLastCalledWith(`${LIST_ID}-callout-0`);
    press("ArrowDown");
    expect(onActive).toHaveBeenLastCalledWith(`${LIST_ID}-callout-1`);
    expect(
      document
        .querySelector(`#${LIST_ID}-callout-1`)
        ?.getAttribute("aria-selected"),
    ).toBe("true");
  });

  it("wraps ArrowUp and ArrowDown around the list", () => {
    const { press } = setup();
    expect(press("ArrowUp")).toBe(true);
    expect(selected()).toContain("Note callout");
    expect(press("ArrowDown")).toBe(true);
    expect(selected()).toContain("Info callout");
  });

  it.each(["Enter", "Tab"])("inserts the highlighted block on %s", (key) => {
    const { command, press } = setup();
    press("ArrowDown");
    expect(press(key)).toBe(true);
    expect(command).toHaveBeenCalledWith(
      expect.objectContaining({ id: "callout-1" }),
    );
  });

  it("leaves other keys to the editor", () => {
    const { command, press } = setup();
    expect(press("a")).toBe(false);
    expect(command).not.toHaveBeenCalled();
  });

  it("follows the mouse and inserts on press without taking focus", () => {
    const { command } = setup();
    const warning = screen.getByRole("option", { name: /Warning callout/ });
    fireEvent.mouseEnter(warning);
    expect(selected()).toContain("Warning callout");
    const pressed = fireEvent.mouseDown(warning);
    expect(pressed).toBe(false);
    expect(command).toHaveBeenCalledWith(
      expect.objectContaining({ id: "callout-1" }),
    );
  });

  it("starts again at the first match when the query changes", () => {
    const { press, rerender, command, onActive } = setup();
    press("ArrowDown");
    rerender(
      <SlashMenu
        items={filterSlashItems("head")}
        command={command}
        t={english}
        listId={LIST_ID}
        onActive={onActive}
      />,
    );
    expect(selected()).toContain("Heading 1");
  });

  it("shows an empty state and handles no keys when nothing matches", () => {
    const { onActive, press } = setup([]);
    expect(screen.getByRole("status").textContent).toBe("No block matches.");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onActive.mock.lastCall).toEqual([undefined]);
    expect(press("Enter")).toBe(false);
  });
});
