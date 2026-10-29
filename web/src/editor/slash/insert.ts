// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Editor, JSONContent, Range } from "@tiptap/core";
import { TextSelection } from "@tiptap/pm/state";

import type { SlashItem } from "./items";
import { TEMPLATES } from "./templates";
import { formatToday } from "./today";

const LIST_COMMANDS: Readonly<Record<string, (editor: Editor) => boolean>> = {
  bulletList: (editor) => editor.chain().focus().toggleBulletList().run(),
  orderedList: (editor) => editor.chain().focus().toggleOrderedList().run(),
  taskList: (editor) => editor.chain().focus().toggleTaskList().run(),
};

/** Replaces the `/query` text with the chosen block. */
export function insertSlashItem(editor: Editor, range: Range, item: SlashItem): boolean {
  editor.chain().focus().deleteRange(range).run();
  const type = editor.schema.nodes[item.node];
  if (!type) {
    return false;
  }
  const list = LIST_COMMANDS[item.node];
  if (list) {
    return list(editor);
  }
  if (item.node === "date") {
    return editor
      .chain()
      .focus()
      .insertContent({ type: "date", attrs: { date: formatToday() } })
      .run();
  }
  if (type.isTextblock && item.template === undefined) {
    return editor.chain().focus().setNode(item.node, item.attrs).run();
  }
  if (item.node === "table") {
    return editor.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run();
  }
  const template = item.template === undefined ? undefined : TEMPLATES[item.template];
  const json: JSONContent | undefined = template?.();
  const node = json
    ? editor.schema.nodeFromJSON({ ...json, attrs: { ...json.attrs, ...item.attrs } })
    : type.createAndFill(item.attrs);
  if (!node) {
    return false;
  }
  return editor
    .chain()
    .focus()
    .command(({ tr }) => {
      const { $from } = tr.selection;
      const empty = $from.parent.isTextblock && $from.parent.content.size === 0 && $from.depth > 0;
      const from = empty ? $from.before() : $from.after();
      const to = $from.after();
      tr.replaceWith(from, to, node);
      const inside = tr.doc.resolve(Math.min(from + 1, tr.doc.content.size));
      tr.setSelection(TextSelection.near(inside));
      return true;
    })
    .run();
}
