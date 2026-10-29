// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { getSchema, type JSONContent } from "@tiptap/core";
import { Collaboration } from "@tiptap/extension-collaboration";
import { CollaborationCaret } from "@tiptap/extension-collaboration-caret";
import { EditorContent, useEditor } from "@tiptap/react";
import { useEffect, useMemo, useState } from "react";
import * as Y from "yjs";

import { useT } from "../i18n/i18n";

import type { CollabConnector, CollabSession, CollabUser } from "./collab/provider";
import { FIELD, seedUpdate } from "./collab/seed";
import { vellumExtensions } from "./extensions";
import type { ResolvePage } from "./nodes/inline";
import { Toolbar } from "./toolbar";

/** Props of {@link PageEditor}. */
export interface PageEditorProperties {
  readonly pageId: string;
  /** Content for a page whose collab document has never been written. */
  readonly seed: JSONContent | undefined;
  readonly connector: CollabConnector;
  readonly user: CollabUser;
  readonly resolvePage: ResolvePage;
  readonly onOpenPage: (id: string) => void;
}

interface BoundProperties extends Omit<PageEditorProperties, "connector" | "pageId"> {
  readonly session: CollabSession;
}

function BoundEditor({ session, seed, user, resolvePage, onOpenPage }: BoundProperties) {
  const t = useT();
  const base = useMemo(
    () => vellumExtensions({ t, resolvePage, history: false }),
    [t, resolvePage],
  );
  const [seeded] = useState(() => {
    if (seed) {
      Y.applyUpdate(session.doc, seedUpdate(getSchema(base), seed));
    }
    return true;
  });
  const editor = useEditor(
    {
      extensions: [
        ...base,
        Collaboration.configure({ document: session.doc, field: FIELD }),
        CollaborationCaret.configure({ provider: session, user }),
      ],
      editorProps: {
        attributes: {
          class: "vl-prose",
          role: "textbox",
          "aria-multiline": "true",
          "aria-label": t("editor-label"),
          spellcheck: "true",
        },
        handleClickOn: (_view, _pos, node) => {
          if (node.type.name !== "wikiLink") {
            return false;
          }
          const page = resolvePage(String(node.attrs["target"]));
          if (page) {
            onOpenPage(page.id);
          }
          return true;
        },
        handleDOMEvents: {
          click: (_view, event) => {
            const target = event.target as HTMLElement | null;
            if (target?.closest("a.vl-wikilink")) {
              event.preventDefault();
            }
            return false;
          },
        },
      },
    },
    [session, seeded],
  );

  return (
    <div className="vl-editor">
      <Toolbar editor={editor} />
      <EditorContent editor={editor} />
    </div>
  );
}

/**
 * The page editor: Tiptap bound to the page's Yjs document. Opening the
 * document is an effect so React's development double-mount never leaks
 * a session.
 */
export function PageEditor({ connector, pageId, ...rest }: PageEditorProperties) {
  const t = useT();
  const [open, setOpen] = useState<{ pageId: string; session: CollabSession }>();
  useEffect(() => {
    const session = connector.open(pageId);
    let live = true;
    void session.ready.then(() => {
      if (live) {
        setOpen({ pageId, session });
      }
    });
    return () => {
      live = false;
      session.destroy();
    };
  }, [connector, pageId]);
  if (open?.pageId !== pageId) {
    return (
      <p className="vl-muted" role="status">
        {t("editor-loading")}
      </p>
    );
  }
  return <BoundEditor key={pageId} session={open.session} {...rest} />;
}
