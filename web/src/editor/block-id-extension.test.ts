// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { Editor, type JSONContent } from "@tiptap/core";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import { StarterKit } from "@tiptap/starter-kit";
import { ySyncPluginKey } from "@tiptap/y-tiptap";
import { afterEach, describe, expect, it } from "vitest";

import { isBlockId } from "./block-id";
import { BlockIds } from "./block-id-extension";
import { body, h, p } from "./content";
import { createTestEditor, typeText } from "./test-editor";

const ID_A = "blk-aaaaaaaa";
const ID_B = "blk-bbbbbbbb";

const withId = (node: JSONContent, blockId: string): JSONContent => ({
  ...node,
  attrs: { ...node.attrs, blockId },
});

/** The block ID of every paragraph and heading, in document order. */
function blockIds(editor: Editor): unknown[] {
  const ids: unknown[] = [];
  editor.state.doc.descendants((node: ProseMirrorNode) => {
    if (node.isTextblock) {
      ids.push(node.attrs["blockId"]);
    }
  });
  return ids;
}

let editor: Editor | undefined;
const open = (content: JSONContent) => {
  editor = createTestEditor({ content });
  return editor;
};
afterEach(() => {
  editor?.destroy();
  editor = undefined;
});

describe("BlockIds", () => {
  it("assigns a blk- id to every block once the document changes", () => {
    const target = open(body(p("one"), h(2, "two")));
    target.commands.insertContentAt(target.state.doc.content.size, p("three"));
    const ids = blockIds(target);
    expect(ids).toHaveLength(3);
    expect(ids.every((id) => isBlockId(id))).toBe(true);
    expect(new Set(ids).size).toBe(3);
  });

  it("keeps existing ids across ordinary edits", () => {
    const target = open(body(withId(p("one"), ID_A), withId(p("two"), ID_B)));
    target.commands.setTextSelection(4);
    typeText(target, " more");
    expect(target.state.doc.firstChild?.textContent).toBe("one more");
    expect(blockIds(target)).toEqual([ID_A, ID_B]);
  });

  it("gives the second half of a split paragraph a fresh id", () => {
    const target = open(body(withId(p("left right"), ID_A)));
    target.commands.setTextSelection(6);
    target.commands.splitBlock();
    const [first, second] = blockIds(target);
    expect(first).toBe(ID_A);
    expect(isBlockId(second)).toBe(true);
    expect(second).not.toBe(ID_A);
  });

  it("re-assigns a duplicated id, as after a paste of a copied block", () => {
    const target = open(body(withId(p("original"), ID_A)));
    target.commands.insertContentAt(
      target.state.doc.content.size,
      withId(p("copy"), ID_A),
    );
    const [first, second] = blockIds(target);
    expect(first).toBe(ID_A);
    expect(isBlockId(second)).toBe(true);
    expect(second).not.toBe(ID_A);
  });

  it("replaces ids that are not canonical", () => {
    const target = open(body(withId(p("odd"), "blk-NOPE")));
    target.commands.insertContent("!");
    expect(isBlockId(blockIds(target)[0])).toBe(true);
  });

  it("leaves changes from peers (y-sync) alone", () => {
    const target = open(body(withId(p("mine"), ID_A)));
    const paragraph = target.schema.nodes["paragraph"]?.create();
    if (!paragraph) {
      throw new Error("no paragraph type");
    }
    const tr = target.state.tr.insert(target.state.doc.content.size, paragraph);
    target.view.dispatch(tr.setMeta(ySyncPluginKey, { isChangeOrigin: true }));
    expect(blockIds(target).map((id) => id ?? "none")).toEqual([ID_A, "none"]);
  });

  it("does not touch the document on a selection-only transaction", () => {
    const target = open(body(p("plain")));
    target.commands.setTextSelection(2);
    expect(blockIds(target).map((id) => id ?? "none")).toEqual(["none"]);
  });

  it("draws again when the generator repeats an id in use", () => {
    const sequence = [ID_A, ID_A, ID_B];
    const target = new Editor({
      extensions: [
        StarterKit,
        BlockIds.configure({
          generate: () => sequence.shift() ?? "blk-cccccccc",
        }),
      ],
      content: body(p("one"), p("two")),
    });
    editor = target;
    target.commands.insertContent("!");
    expect(blockIds(target)).toEqual([ID_A, ID_B]);
  });

  it("round-trips ids through HTML as data-block-id", () => {
    const target = open(body(p("x")));
    target.commands.setContent(`<p data-block-id="${ID_A}">x</p><p>y</p>`);
    const html = target.getHTML();
    expect(html).toContain(`data-block-id="${ID_A}"`);
    expect(blockIds(target)[0]).toBe(ID_A);
  });
});
