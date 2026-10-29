// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { body, p, tabs } from "../content";
import { createTestEditor, detachedViewProperties, renderEditor } from "../test-editor";

import { TabsView } from "./tabs-view";

const content = body(tabs(["Overview", p("first")], ["Details", p("second")], ["", p("third")]));

const selectedTab = () =>
  screen.getAllByRole("tab").find((tab) => tab.getAttribute("aria-selected") === "true");

/** The visible panel's text, without its title field. */
const visiblePanelText = () =>
  screen
    .getAllByRole("tabpanel", { hidden: true })
    .filter((panel) => !panel.hidden)
    .map((panel) => panel.textContent);

describe("TabsView", () => {
  it("shows a tab list with the first tab selected and its panel visible", async () => {
    await renderEditor(content);
    expect(screen.getByRole("tablist", { name: "Tabs" })).toBeDefined();
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Overview",
      "Details",
      "Untitled tab",
    ]);
    expect(selectedTab()?.textContent).toBe("Overview");
    expect(visiblePanelText()).toEqual(["first"]);
  });

  it("links each tab to its panel both ways", async () => {
    await renderEditor(content);
    const [tab] = screen.getAllByRole("tab");
    const panel = document.querySelector(
      `#${CSS.escape(tab?.getAttribute("aria-controls") ?? "")}`,
    );
    expect(panel?.getAttribute("aria-labelledby")).toBe(tab?.id);
  });

  it("keeps only the selected tab in the tab order", async () => {
    await renderEditor(content);
    expect(screen.getAllByRole("tab").map((tab) => tab.tabIndex)).toEqual([0, -1, -1]);
  });

  it("selects a tab on click", async () => {
    const user = userEvent.setup();
    await renderEditor(content);
    await user.click(screen.getByRole("tab", { name: "Details" }));
    expect(selectedTab()?.textContent).toBe("Details");
    expect(visiblePanelText()).toEqual(["second"]);
  });

  it("moves with the arrow keys, wrapping at both ends, and Home and End", async () => {
    const user = userEvent.setup();
    await renderEditor(content);
    screen.getByRole("tab", { name: "Overview" }).focus();
    await user.keyboard("{ArrowRight}");
    expect(document.activeElement?.textContent).toBe("Details");
    expect(selectedTab()?.textContent).toBe("Details");
    await user.keyboard("{ArrowRight}{ArrowRight}");
    expect(selectedTab()?.textContent).toBe("Overview");
    await user.keyboard("{ArrowLeft}");
    expect(selectedTab()?.textContent).toBe("Untitled tab");
    await user.keyboard("{Home}");
    expect(document.activeElement?.textContent).toBe("Overview");
    await user.keyboard("{End}");
    expect(selectedTab()?.textContent).toBe("Untitled tab");
    expect(visiblePanelText()).toEqual(["third"]);
  });

  it("ignores other keys", async () => {
    const user = userEvent.setup();
    await renderEditor(content);
    screen.getByRole("tab", { name: "Overview" }).focus();
    await user.keyboard("{ArrowDown}x");
    expect(selectedTab()?.textContent).toBe("Overview");
  });

  it("adds a numbered tab at the end and selects it", async () => {
    const user = userEvent.setup();
    const { editor } = await renderEditor(content);
    await user.click(screen.getByRole("button", { name: "Add a tab" }));
    expect(editor.state.doc.firstChild?.lastChild?.attrs["title"]).toBe("Tab 4");
    expect(selectedTab()?.textContent).toBe("Tab 4");
  });

  it("renames a tab from its title field", async () => {
    const user = userEvent.setup();
    const { editor } = await renderEditor(content);
    const [title] = screen.getAllByRole("textbox", { name: "Tab title" });
    if (!title) {
      throw new Error("no title field");
    }
    await user.clear(title);
    await user.type(title, "Summary");
    expect(editor.state.doc.firstChild?.firstChild?.attrs["title"]).toBe("Summary");
    expect(screen.getByRole("tab", { name: "Summary" })).toBeDefined();
  });

  it("offers no add button and read-only titles in a read-only editor", async () => {
    await renderEditor(content, { editable: false });
    expect(screen.queryByRole("button", { name: "Add a tab" })).toBeNull();
    const titles = screen.getAllByRole("textbox", { name: "Tab title" });
    expect(titles.every((title) => (title as HTMLInputElement).readOnly)).toBe(true);
  });

  it("adds nothing once the block has left the document", async () => {
    const user = userEvent.setup();
    const editor = createTestEditor({ content });
    const node = editor.state.doc.firstChild;
    if (!node) {
      throw new Error("no tabs block");
    }
    render(<TabsView {...detachedViewProperties(editor, node)} />);
    await user.click(screen.getByRole("button", { name: "Add a tab" }));
    expect(editor.state.doc.firstChild?.childCount).toBe(3);
    expect(selectedTab()?.textContent).toBe("Overview");
    editor.destroy();
  });
});
