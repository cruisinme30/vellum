// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, render, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import type { Editor } from "@tiptap/core";
import { afterEach, describe, expect, it, vi } from "vitest";

import { body, link, p } from "./content";
import { renderEditor } from "./test-editor";
import { Toolbar } from "./toolbar";

async function setup(content = body(p("Hello world"))) {
  const user = userEvent.setup();
  const { editor } = await renderEditor(content);
  render(<Toolbar editor={editor} />);
  act(() => {
    editor.commands.setTextSelection({ from: 1, to: 6 });
  });
  return { user, editor };
}

const marksOnHello = (editor: Editor) =>
  (editor.state.doc.firstChild?.firstChild?.marks ?? []).map(
    (mark) => mark.type.name,
  );

afterEach(() => {
  vi.restoreAllMocks();
});

describe("Toolbar", () => {
  it("is one labelled toolbar with a single tab stop", async () => {
    await setup();
    const toolbar = screen.getByRole("toolbar", { name: "Formatting" });
    const tabbable = [...toolbar.querySelectorAll("button")].filter(
      (button) => button.tabIndex === 0,
    );
    expect(tabbable.map((button) => button.getAttribute("aria-label"))).toEqual(
      ["Bold"],
    );
    expect(
      screen
        .getByRole("button", { name: "Bold" })
        .getAttribute("aria-keyshortcuts"),
    ).toBe("Control+B");
  });

  it("moves focus with the arrow keys and wraps around", async () => {
    const { user } = await setup();
    screen.getByRole("button", { name: "Bold" }).focus();
    await user.keyboard("{ArrowRight}");
    expect(document.activeElement?.getAttribute("aria-label")).toBe("Italic");
    await user.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(document.activeElement?.textContent).toBe("Insert");
    await user.keyboard("{ArrowRight}");
    expect(document.activeElement?.getAttribute("aria-label")).toBe("Bold");
    await user.keyboard("{ArrowDown}");
    expect(document.activeElement?.getAttribute("aria-label")).toBe("Bold");
  });

  it.each([
    ["Bold", "bold"],
    ["Italic", "italic"],
    ["Strikethrough", "strike"],
    ["Inline code", "code"],
    ["Highlight", "highlight"],
  ])(
    "toggles %s on the selection and shows it as pressed",
    async (label, mark) => {
      const { user, editor } = await setup();
      const button = screen.getByRole("button", { name: label });
      expect(button.getAttribute("aria-pressed")).toBe("false");
      await user.click(button);
      expect(marksOnHello(editor)).toEqual([mark]);
      expect(button.getAttribute("aria-pressed")).toBe("true");
      await user.click(button);
      expect(marksOnHello(editor)).toEqual([]);
    },
  );

  it("links the selection to a safe address from the prompt", async () => {
    const { user, editor } = await setup();
    const prompt = vi
      .spyOn(globalThis, "prompt")
      .mockReturnValue("https://example.com");
    await user.click(screen.getByRole("button", { name: "Link" }));
    expect(prompt).toHaveBeenCalledWith(
      "Link address (leave empty to remove the link)",
      "",
    );
    expect(editor.getAttributes("link")["href"]).toBe("https://example.com/");
    expect(
      screen.getByRole("button", { name: "Link" }).getAttribute("aria-pressed"),
    ).toBe("true");
  });

  it("offers the current address and removes the link when it is cleared", async () => {
    const { user, editor } = await setup(
      body(p(link("Hello", "https://old.example"), " world")),
    );
    const prompt = vi.spyOn(globalThis, "prompt").mockReturnValue("");
    await user.click(screen.getByRole("button", { name: "Link" }));
    expect(prompt).toHaveBeenCalledWith(
      expect.any(String),
      "https://old.example",
    );
    expect(marksOnHello(editor)).toEqual([]);
  });

  it("refuses an unsafe address by removing the link", async () => {
    const { user, editor } = await setup(
      body(p(link("Hello", "https://old.example"))),
    );
    vi.spyOn(globalThis, "prompt").mockReturnValue("javascript:alert(1)");
    await user.click(screen.getByRole("button", { name: "Link" }));
    expect(marksOnHello(editor)).toEqual([]);
  });

  it("changes nothing when the prompt is cancelled", async () => {
    const { user, editor } = await setup();
    // eslint-disable-next-line unicorn/no-null -- window.prompt returns null on cancel
    vi.spyOn(globalThis, "prompt").mockReturnValue(null);
    const before = editor.getJSON();
    await user.click(screen.getByRole("button", { name: "Link" }));
    expect(editor.getJSON()).toEqual(before);
  });

  it("opens the block menu from Insert", async () => {
    const { user, editor } = await setup(body(p()));
    await user.click(screen.getByRole("button", { name: "Insert" }));
    expect(editor.state.doc.textContent).toBe("/");
    expect(
      await screen.findByRole("listbox", { name: "Blocks" }),
    ).toBeDefined();
  });

  it("makes a task from the selection, and only with something selected", async () => {
    const user = userEvent.setup();
    const { editor } = await renderEditor(body(p("Hello world")));
    const onSelectionTask = vi.fn();
    render(<Toolbar editor={editor} onSelectionTask={onSelectionTask} />);
    const button = screen.getByRole("button", {
      name: "Make a task from the selection",
    });
    expect(button.getAttribute("aria-keyshortcuts")).toBe("Control+Alt+Enter");
    expect(button).toHaveProperty("disabled", true);
    act(() => {
      editor.commands.setTextSelection({ from: 1, to: 6 });
    });
    expect(button).toHaveProperty("disabled", false);
    await user.click(button);
    expect(onSelectionTask).toHaveBeenCalledOnce();
  });
});
