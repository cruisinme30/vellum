// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * Turning selected page text into a task (R-MW-3, D248): a modifier key or
 * the toolbar while editing, a bare `T` only on a page that cannot be edited.
 */

import type { Editor } from "@tiptap/core";
import type { EditorState } from "@tiptap/pm/state";
import { useEffect } from "react";

import { isEditable } from "../keyboard/keymap";

/** Selected text as one line, or `undefined` when only space is selected. */
function oneLine(text: string): string | undefined {
  const line = text.trim().replaceAll(/\s+/g, " ");
  return line === "" ? undefined : line;
}

/** Whether a key press is "make a task from the selection": Ctrl+Alt+Enter, or Cmd+Option+Enter on a Mac. */
export const isSelectionTaskKey = (event: KeyboardEvent) =>
  (event.ctrlKey || event.metaKey) && event.altKey && !event.shiftKey && event.key === "Enter";

/**
 * The selected text, outside code, that the editor's "make a task" key or
 * button turns into a task (R-MW-3); `undefined` with nothing to use.
 */
export function selectionTaskText(state: EditorState): string | undefined {
  const { selection } = state;
  if (selection.empty) return undefined;
  if (selection.$from.parent.type.spec.code === true) return undefined;
  return oneLine(state.doc.textBetween(selection.from, selection.to, " ", " "));
}

/**
 * On a page that cannot be edited, a bare `T` turns the text selected in
 * `root` into a task. Typing never reaches a read-only page, so the plain
 * key is safe there; in the editor it stays a letter.
 */
export function readOnlyTaskText(event: KeyboardEvent, root: Element): string | undefined {
  if (event.key !== "t" && event.key !== "T") return undefined;
  if (event.metaKey || event.ctrlKey || event.altKey || event.isComposing) return undefined;
  if (isEditable(event.target)) return undefined;
  const selection = globalThis.getSelection();
  if (selection === null || selection.isCollapsed || selection.rangeCount === 0) return undefined;
  if (!root.contains(selection.getRangeAt(0).commonAncestorContainer)) return undefined;
  return oneLine(selection.toString());
}

/** On a page that cannot be edited, listens for `T` over a selection in it. */
export function useReadOnlyTaskKey(
  editor: Editor,
  editable: boolean,
  onSelectionTask: ((text: string) => void) | undefined,
) {
  useEffect(() => {
    if (editable || onSelectionTask === undefined) return;
    const onKeyDown = (event: KeyboardEvent) => {
      const selected = readOnlyTaskText(event, editor.view.dom);
      if (selected === undefined) return;
      event.preventDefault();
      onSelectionTask(selected);
    };
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [editor, editable, onSelectionTask]);
}
