// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Editor } from "@tiptap/core";
import { NodeViewWrapper, type ReactNodeViewProps } from "@tiptap/react";
import { useEffect, useState } from "react";

import { useT } from "../../i18n/i18n";

/** A heading the table of contents links to. */
export interface TocEntry {
  readonly level: number;
  readonly text: string;
  readonly pos: number;
}

/** The headings after a position, in order. */
export function headingsAfter(editor: Editor, from: number): TocEntry[] {
  const out: TocEntry[] = [];
  editor.state.doc.descendants((node, pos) => {
    if (node.type.name === "heading" && pos > from && node.textContent.trim() !== "") {
      out.push({ level: Number(node.attrs["level"]), text: node.textContent, pos });
    }
    return node.isBlock && node.type.name !== "heading";
  });
  return out;
}

/** `::: toc`: links to the headings below it, kept current as the page changes. */
export function TocView({ editor, getPos }: ReactNodeViewProps) {
  const t = useT();
  const read = () => headingsAfter(editor, getPos() ?? 0);
  const [entries, setEntries] = useState(read);
  useEffect(() => {
    const refresh = () => {
      setEntries(headingsAfter(editor, getPos() ?? 0));
    };
    editor.on("update", refresh);
    return () => {
      editor.off("update", refresh);
    };
  }, [editor, getPos]);

  return (
    <NodeViewWrapper
      as="nav"
      className="vl-block vl-toc"
      aria-label={t("toc-label")}
      contentEditable={false}
    >
      {entries.length === 0 ? (
        <p className="vl-muted">{t("toc-empty")}</p>
      ) : (
        <ul>
          {entries.map((entry) => (
            <li key={entry.pos} data-level={entry.level}>
              <button
                type="button"
                className="vl-link-button"
                onClick={() => {
                  editor
                    .chain()
                    .focus()
                    .setTextSelection(entry.pos + 1)
                    .scrollIntoView()
                    .run();
                }}
              >
                {entry.text}
              </button>
            </li>
          ))}
        </ul>
      )}
    </NodeViewWrapper>
  );
}
