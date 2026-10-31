// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Editor } from "@tiptap/core";
import { useEditorState } from "@tiptap/react";
import type { KeyboardEvent } from "react";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";

import { safeUrl } from "./urls";

interface Action {
  readonly id: string;
  readonly icon: string;
  readonly mark: string;
  readonly keys: string;
  readonly run: (editor: Editor) => void;
}

const ACTIONS: readonly Action[] = [
  {
    id: "bold",
    icon: "format_bold",
    mark: "bold",
    keys: "Mod+B",
    run: (target) => target.chain().focus().toggleBold().run(),
  },
  {
    id: "italic",
    icon: "format_italic",
    mark: "italic",
    keys: "Mod+I",
    run: (target) => target.chain().focus().toggleItalic().run(),
  },
  {
    id: "strike",
    icon: "format_strikethrough",
    mark: "strike",
    keys: "Mod+Shift+S",
    run: (target) => target.chain().focus().toggleStrike().run(),
  },
  {
    id: "code",
    icon: "code",
    mark: "code",
    keys: "Mod+E",
    run: (target) => target.chain().focus().toggleCode().run(),
  },
  {
    id: "highlight",
    icon: "format_ink_highlighter",
    mark: "highlight",
    keys: "Mod+Shift+H",
    run: (target) => target.chain().focus().toggleHighlight().run(),
  },
];

/**
 * Arrow keys move focus between the toolbar's buttons, wrapping at the ends.
 */
const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
  const buttons = [...event.currentTarget.querySelectorAll("button")];
  const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
  const step: Record<string, number> = { ArrowRight: 1, ArrowLeft: -1 };
  const delta = step[event.key];
  if (delta !== undefined && at !== -1) {
    event.preventDefault();
    buttons[(at + delta + buttons.length) % buttons.length]?.focus();
  }
};

/**
 * The formatting toolbar: one tab stop, arrow keys between buttons (WAI-ARIA
 * toolbar).
 */
export function Toolbar({
  editor,
  onSelectionTask,
}: {
  readonly editor: Editor;
  /** Makes a task from the selected text; no button when absent. */
  readonly onSelectionTask?: (() => void) | undefined;
}) {
  const t = useT();
  const active = useEditorState({
    editor,
    selector: ({ editor: current }) =>
      Object.fromEntries([
        ...ACTIONS.map((a) => [a.id, current.isActive(a.mark)]),
        ["link", current.isActive("link")],
        ["selection", !current.state.selection.empty],
      ]) as Record<string, boolean>,
  });

  const setLink = () => {
    const current = String(editor.getAttributes("link")["href"] ?? "");
    const answer = globalThis.prompt(t("toolbar-link-prompt"), current);
    if (answer === null) {
      return;
    }
    const href = safeUrl(answer);
    if (href === undefined) {
      editor.chain().focus().extendMarkRange("link").unsetLink().run();
    } else {
      editor.chain().focus().extendMarkRange("link").setLink({ href }).run();
    }
  };

  return (
    <div
      className="vl-toolbar"
      role="toolbar"
      aria-label={t("toolbar-label")}
      onKeyDown={onKeyDown}
    >
      {ACTIONS.map((action, index) => (
        <button
          key={action.id}
          type="button"
          className="vl-icon-button"
          tabIndex={index === 0 ? 0 : -1}
          aria-pressed={active[action.id] === true}
          aria-label={t(`toolbar-${action.id}`)}
          aria-keyshortcuts={action.keys.replace("Mod", "Control")}
          title={t(`toolbar-${action.id}`)}
          onClick={() => {
            action.run(editor);
          }}
        >
          <Icon name={action.icon} />
        </button>
      ))}
      <button
        type="button"
        className="vl-icon-button"
        tabIndex={-1}
        aria-pressed={active["link"] === true}
        aria-label={t("toolbar-link")}
        title={t("toolbar-link")}
        onClick={setLink}
      >
        <Icon name="add_link" />
      </button>
      {onSelectionTask !== undefined && (
        <button
          type="button"
          className="vl-icon-button"
          tabIndex={-1}
          disabled={active["selection"] !== true}
          aria-label={t("toolbar-task")}
          aria-keyshortcuts="Control+Alt+Enter"
          title={t("toolbar-task")}
          onClick={onSelectionTask}
        >
          <Icon name="checklist" />
        </button>
      )}
      <span className="vl-toolbar-gap" />
      <button
        type="button"
        className="vl-text-button"
        tabIndex={-1}
        onClick={() => {
          editor.chain().focus().insertContent("/").run();
        }}
      >
        <Icon name="add" />
        {t("toolbar-insert")}
      </button>
    </div>
  );
}
