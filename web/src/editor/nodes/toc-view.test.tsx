// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { act, render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { body, callout, h, p, toc } from "../content";
import { createTestEditor, detachedViewProperties, renderEditor } from "../test-editor";

import { headingsAfter, TocView } from "./toc-view";

const entries = () =>
  within(screen.getByRole("navigation", { name: "Table of contents" }))
    .queryAllByRole("listitem")
    .map((item) => [item.dataset["level"], item.textContent]);

describe("TocView", () => {
  it("lists the non-empty headings below it, nested blocks included", async () => {
    await renderEditor(
      body(
        h(1, "Before"),
        toc(),
        h(2, "Purpose"),
        p("text"),
        h(3, "  "),
        callout("info", h(3, "Inside a callout")),
        h(2, "Scope"),
      ),
    );
    expect(entries()).toEqual([
      ["2", "Purpose"],
      ["3", "Inside a callout"],
      ["2", "Scope"],
    ]);
  });

  it("explains itself when there is no heading below", async () => {
    await renderEditor(body(toc(), p("just text")));
    expect(screen.getByText("Headings below this block appear here.")).toBeDefined();
    expect(entries()).toEqual([]);
  });

  it("follows edits to the page", async () => {
    const { editor } = await renderEditor(body(toc(), h(2, "Old")));
    act(() => {
      editor.commands.insertContentAt(editor.state.doc.content.size, h(2, "New"));
    });
    expect(entries()).toEqual([
      ["2", "Old"],
      ["2", "New"],
    ]);
  });

  it("moves the cursor into a heading when its entry is clicked", async () => {
    const user = userEvent.setup();
    const { editor } = await renderEditor(body(toc(), p("intro"), h(2, "Target")));
    await user.click(screen.getByRole("button", { name: "Target" }));
    expect(editor.state.selection.$from.parent.textContent).toBe("Target");
  });

  it("stops listening when unmounted", async () => {
    const { editor, unmount } = await renderEditor(body(toc(), h(2, "One")));
    unmount();
    expect(() => editor.commands.insertContent("x")).not.toThrow();
  });
});

describe("TocView outside the document", () => {
  it("lists the headings from the top of the page when it has no position", () => {
    const editor = createTestEditor({ content: body(p("x"), h(2, "Top"), toc(), h(2, "Below")) });
    const node = editor.state.doc.child(2);
    render(<TocView {...detachedViewProperties(editor, node)} />);
    expect(entries()).toEqual([
      ["2", "Top"],
      ["2", "Below"],
    ]);
    act(() => {
      editor.commands.insertContentAt(3, h(3, "First"));
    });
    expect(entries()[0]).toEqual(["3", "First"]);
    editor.destroy();
  });
});

describe("headingsAfter", () => {
  it("returns headings after a position with their level and position", () => {
    const editor = createTestEditor({ content: body(h(1, "A"), p("b"), h(2, "C")) });
    expect(headingsAfter(editor, 0)).toEqual([{ level: 2, text: "C", pos: 6 }]);
    expect(headingsAfter(editor, -1).map((entry) => entry.text)).toEqual(["A", "C"]);
    editor.destroy();
  });
});
