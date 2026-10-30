// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Editor } from "@tiptap/core";
import { afterEach, describe, expect, it } from "vitest";

import { body, p } from "../content";
import { createTestEditor, topLevelTypes } from "../test-editor";

import { insertSlashItem } from "./insert";
import { SLASH_ITEMS, type SlashItem } from "./items";
import { formatToday } from "./today";

let editor: Editor | undefined;
/**
 * An editor holding `before/query` with the cursor after it, and the range of
 * `/query`.
 */
const open = (before = "", query = "/x") => {
  editor = createTestEditor({ content: body(p(`${before}${query}`)) });
  const from = 1 + before.length;
  const to = from + query.length;
  editor.commands.setTextSelection(to);
  return { target: editor, range: { from, to } };
};
afterEach(() => {
  editor?.destroy();
  editor = undefined;
});

const item = (id: string): SlashItem => {
  const found = SLASH_ITEMS.find((entry) => entry.id === id);
  if (!found) {
    throw new Error(`no slash item ${id}`);
  }
  return found;
};

/** Every node type name in the document. */
function typesIn(target: Editor): Set<string> {
  const names = new Set<string>();
  target.state.doc.descendants((node) => {
    names.add(node.type.name);
  });
  return names;
}

describe("insertSlashItem", () => {
  it.each(SLASH_ITEMS.map((entry) => [entry.title, entry] as const))(
    "inserts a %s block in place of the query",
    (_title, entry) => {
      const { target, range } = open();
      expect(insertSlashItem(target, range, entry)).toBe(true);
      expect(typesIn(target).has(entry.node)).toBe(true);
      expect(target.state.doc.textContent).not.toContain("/x");
    },
  );

  it("applies the entry's attributes, such as a heading level", () => {
    const { target, range } = open();
    insertSlashItem(target, range, item("heading-2"));
    expect(target.state.doc.firstChild?.attrs["level"]).toBe(3);
  });

  it("turns an emptied paragraph into the block instead of adding one", () => {
    const { target, range } = open();
    insertSlashItem(target, range, item("callout-1"));
    expect(topLevelTypes(target)).toEqual(["callout"]);
    expect(target.state.doc.firstChild?.attrs["kind"]).toBe("warn");
  });

  it("adds the block after a paragraph that still has text", () => {
    const { target, range } = open("Intro ");
    insertSlashItem(target, range, item("embed-0"));
    expect(topLevelTypes(target)).toEqual(["paragraph", "embed"]);
    expect(target.state.doc.firstChild?.textContent).toBe("Intro ");
  });

  it("fills a block from its template and keeps the entry's attributes", () => {
    const { target, range } = open();
    insertSlashItem(target, range, item("dataFence-5"));
    const fence = target.state.doc.firstChild;
    expect(fence?.type.name).toBe("dataFence");
    expect(fence?.attrs).toMatchObject({ lang: "table", args: "sortable" });
    expect(fence?.textContent).toContain("Alpha, 120");
  });

  it("puts the cursor inside the new block", () => {
    const { target, range } = open();
    insertSlashItem(target, range, item("details-0"));
    expect(target.state.selection.$from.parent.type.name).toBe("paragraph");
    expect(target.state.selection.$from.depth).toBeGreaterThan(1);
  });

  it("inserts a three by three table with a header row", () => {
    const { target, range } = open();
    insertSlashItem(target, range, item("table-0"));
    const table = target.state.doc.firstChild;
    expect(table?.childCount).toBe(3);
    expect(table?.firstChild?.firstChild?.type.name).toBe("tableHeader");
    expect(table?.firstChild?.childCount).toBe(3);
  });

  it("inserts today's date as a chip", () => {
    const { target, range } = open("Due ");
    insertSlashItem(target, range, item("date-0"));
    expect(target.getJSON().content[0]?.content?.[1]).toEqual({
      type: "date",
      attrs: { date: formatToday() },
    });
  });

  it("wraps the paragraph in a list", () => {
    const { target, range } = open("Buy milk");
    insertSlashItem(target, range, item("taskList-0"));
    expect(topLevelTypes(target)).toEqual(["taskList"]);
    expect(target.state.doc.textContent).toBe("Buy milk");
  });

  it("does nothing but remove the query for a node the schema lacks", () => {
    const { target, range } = open("a");
    expect(
      insertSlashItem(target, range, { ...item("toc-0"), node: "nope" }),
    ).toBe(false);
    expect(target.state.doc.textContent).toBe("a");
  });
});
