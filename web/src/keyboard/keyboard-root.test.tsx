// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { setTelemetrySink, type TelemetrySink } from "../platform/telemetry";

import { KeyboardRoot } from "./keyboard-root";
import { workCommands } from "./work-commands";

afterEach(() => {
  cleanup();
});

function key(name: string, extra: Partial<KeyboardEventInit> = {}) {
  const code = /^[a-z]$/i.test(name) ? `Key${name.toUpperCase()}` : undefined;
  act(() => {
    fireEvent.keyDown(document.activeElement ?? document.body, {
      key: name,
      ...(code ? { code } : {}),
      ...extra,
    });
  });
}

function setup(scopes: ("issues" | "editor")[] = []) {
  const actions = { newIssue: vi.fn(), goTo: vi.fn(), assignToMe: vi.fn() };
  render(
    <KeyboardRoot
      commands={workCommands(actions)}
      scopes={scopes}
      platform="mac"
    >
      <button type="button">Somewhere</button>
      <input aria-label="Title" />
    </KeyboardRoot>,
  );
  return actions;
}

describe("KeyboardRoot", () => {
  it("runs shortcuts and chords from anywhere on the page", () => {
    const actions = setup();
    key("c");
    expect(actions.newIssue).toHaveBeenCalledOnce();
    key("g");
    key("i");
    expect(actions.goTo).toHaveBeenCalledWith("inbox");
    // Issue-list keys do nothing outside an issue list.
    key("i");
    expect(actions.assignToMe).not.toHaveBeenCalled();
  });

  it("leaves typing in fields alone", () => {
    const actions = setup();
    screen.getByRole("textbox", { name: "Title" }).focus();
    key("c");
    expect(actions.newIssue).not.toHaveBeenCalled();
  });

  it("applies issue-list keys on an issue list", () => {
    const actions = setup(["issues"]);
    key("i");
    expect(actions.assignToMe).toHaveBeenCalledOnce();
  });

  it("finds and runs a command from the ⌘K menu", () => {
    const actions = setup();
    const button = screen.getByRole("button", { name: "Somewhere" });
    button.focus();
    key("k", { metaKey: true });
    const search = screen.getByRole("combobox", { name: "Search commands" });
    expect(document.activeElement).toBe(search);
    fireEvent.change(search, { target: { value: "inbox" } });
    expect(screen.getAllByRole("option")[0]?.textContent).toContain(
      "Go to Inbox",
    );
    expect(screen.getAllByRole("option")[0]?.textContent).toContain("G then I");
    fireEvent.keyDown(search, { key: "Enter" });
    expect(actions.goTo).toHaveBeenCalledWith("inbox");
    expect(screen.queryByRole("dialog")).toBeNull();
    // Focus goes back where it was.
    expect(document.activeElement).toBe(button);
  });

  it("moves through results with the arrows and runs one by click", () => {
    const actions = setup();
    key("k", { metaKey: true });
    const search = screen.getByRole("combobox");
    fireEvent.change(search, { target: { value: "new issue" } });
    fireEvent.keyDown(search, { key: "ArrowDown" });
    fireEvent.keyDown(search, { key: "ArrowUp" });
    fireEvent.keyDown(search, { key: "Tab" });
    const options = screen.getAllByRole("option");
    expect(options[0]?.getAttribute("aria-selected")).toBe("true");
    expect(search.getAttribute("aria-activedescendant")).toBe(options[0]?.id);
    fireEvent.mouseEnter(options[0] ?? search);
    fireEvent.mouseDown(options[0] ?? search);
    expect(actions.newIssue).toHaveBeenCalledOnce();
  });

  it("says when nothing matches and closes on Escape", () => {
    const actions = setup();
    key("k", { metaKey: true });
    const search = screen.getByRole("combobox");
    fireEvent.change(search, { target: { value: "zzzz" } });
    expect(screen.getByRole("status").textContent).toContain("zzzz");
    fireEvent.keyDown(search, { key: "Enter" });
    fireEvent.keyDown(search, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
    // Shortcuts work again once the menu is closed.
    key("c");
    expect(actions.newIssue).toHaveBeenCalledOnce();
  });

  it("shows the shortcut sheet for the current screen on ?", () => {
    setup(["issues"]);
    key("?", { shiftKey: true });
    const sheet = screen.getByRole("dialog", { name: "Keyboard shortcuts" });
    expect(sheet.textContent).toContain("Everywhere");
    expect(sheet.textContent).toContain("In issue lists");
    expect(sheet.textContent).toContain("⌘K");
    expect(sheet.textContent).not.toContain("In the editor");
    key("Escape");
    expect(screen.queryByRole("dialog")).toBeNull();
    key("?");
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("records how long a shortcut took and which command ran", () => {
    const histogram = vi.fn();
    const count = vi.fn();
    const sink: TelemetrySink = { histogram, count };
    const previous = setTelemetrySink(sink);
    try {
      setup();
      key("c");
      expect(histogram).toHaveBeenCalledWith(
        "ui.keyboard.dispatch",
        expect.any(Number),
        {
          command: "issue.create",
        },
      );
      expect(count).toHaveBeenCalledWith("ui.keyboard.commands", {
        command: "issue.create",
        via: "key",
      });
    } finally {
      setTelemetrySink(previous);
    }
  });

  it("works with no screen commands", () => {
    render(<KeyboardRoot platform="other" />);
    key("k", { ctrlKey: true });
    expect(screen.getByRole("dialog", { name: "Command menu" })).toBeTruthy();
  });
});
