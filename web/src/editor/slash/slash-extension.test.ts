// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, fireEvent, screen } from "@testing-library/react";
import type { Editor } from "@tiptap/core";
import { describe, expect, it } from "vitest";

import { body, p } from "../content";
import { renderEditor, topLevelTypes, typeText } from "../test-editor";

async function openMenu(query = "") {
  const { editor } = await renderEditor(body(p()));
  act(() => {
    editor.commands.focus("end");
    typeText(editor, `/${query}`);
  });
  return editor;
}

const press = (editor: Editor, key: string) => {
  act(() => {
    fireEvent.keyDown(editor.view.dom, { key });
  });
};

describe("SlashCommand", () => {
  it("opens the block menu on / and wires up its ARIA state", async () => {
    const editor = await openMenu();
    const list = await screen.findByRole("listbox", { name: "Blocks" });
    const dom = editor.view.dom;
    expect(dom.getAttribute("aria-expanded")).toBe("true");
    expect(dom.getAttribute("aria-controls")).toBe(list.id);
    expect(dom.getAttribute("aria-activedescendant")).toBe(
      "vl-slash-list-paragraph-0",
    );
    expect(list.parentElement?.style.position).toBe("fixed");
  });

  it("filters as the user types and moves the active option with the arrows", async () => {
    const editor = await openMenu("call");
    expect(await screen.findAllByRole("option")).toHaveLength(6);
    press(editor, "ArrowDown");
    expect(editor.view.dom.getAttribute("aria-activedescendant")).toBe(
      "vl-slash-list-callout-1",
    );
  });

  it("inserts the chosen block on Enter and closes", async () => {
    const editor = await openMenu("toc");
    await screen.findByRole("listbox");
    press(editor, "Enter");
    expect(topLevelTypes(editor)[0]).toBe("toc");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(editor.view.dom.getAttribute("aria-expanded")).toBe("false");
    expect(editor.view.dom.hasAttribute("aria-activedescendant")).toBe(false);
    expect(editor.view.dom.hasAttribute("aria-controls")).toBe(false);
  });

  it("shows an empty state when nothing matches", async () => {
    await openMenu("zzzz");
    const status = await screen.findByRole("status");
    expect(status.textContent).toBe("No block matches.");
  });

  it("closes on Escape and leaves the text", async () => {
    const editor = await openMenu("he");
    await screen.findByRole("listbox");
    press(editor, "Escape");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(editor.state.doc.textContent).toBe("/he");
  });

  it("opens above the cursor when there is no room below", async () => {
    const { innerHeight } = globalThis;
    globalThis.innerHeight = 100;
    try {
      await openMenu();
      const list = await screen.findByRole("listbox");
      const menu = list.parentElement;
      expect(menu?.style.top).toBe("");
      expect(menu?.style.bottom).not.toBe("");
    } finally {
      globalThis.innerHeight = innerHeight;
    }
  });
});
