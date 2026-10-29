// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { getSchema, type JSONContent } from "@tiptap/core";
import { Collaboration } from "@tiptap/extension-collaboration";
import { CollaborationCaret } from "@tiptap/extension-collaboration-caret";
import { EditorContent, useEditor } from "@tiptap/react";
import { useEffect, useMemo, useState, useSyncExternalStore } from "react";
import * as Y from "yjs";

import { useT } from "../i18n/i18n";

import type {
  CollabConnector,
  CollabSession,
  CollabUser,
  LiveConnection,
  LiveState,
} from "./collab/provider";
import { FIELD, seedUpdate } from "./collab/seed";
import { vellumExtensions } from "./extensions";
import type { ResolvePage } from "./nodes/inline";
import { Toolbar } from "./toolbar";

/** Props of {@link PageEditor}. */
export interface PageEditorProperties {
  readonly pageId: string;
  /** The drive space that holds the page; names its live-editing room. */
  readonly spaceId: string;
  /** Content for a page whose collab document has never been written. */
  readonly seed: JSONContent | undefined;
  readonly connector: CollabConnector;
  readonly user: CollabUser;
  readonly resolvePage: ResolvePage;
  readonly onOpenPage: (id: string) => void;
}

interface BoundProperties extends Omit<PageEditorProperties, "connector" | "pageId" | "spaceId"> {
  readonly session: CollabSession;
}

/** A local session: always online and editable, and never changes. */
const unsubscribeNothing = () => {
  // A local session never changes, so there is nothing to unsubscribe.
};
const LOCAL: LiveConnection = {
  getSnapshot: () => LOCAL_STATE,
  subscribe: () => unsubscribeNothing,
  done: () => Promise.resolve(),
};
const LOCAL_STATE: LiveState = { status: "online", canWrite: true, peers: new Map() };

/** The session's server state. */
function useLive(live: LiveConnection = LOCAL): LiveState {
  return useSyncExternalStore(live.subscribe, live.getSnapshot);
}

/** One line under the toolbar when the page is not plainly editable and synced. */
function LiveNotice({ state }: { readonly state: LiveState }) {
  const t = useT();
  let key: string | undefined;
  switch (state.status) {
    case "revoked": {
      key = "editor-revoked";

      break;
    }
    case "offline": {
      key = "editor-offline";

      break;
    }
    case "connecting": {
      key = "editor-connecting";

      break;
    }
    default: {
      if (!state.canWrite) {
        key = "editor-read-only";
      }
    }
  }
  // The live region stays mounted so screen readers announce each change.
  return (
    <div role="status">
      {key === undefined ? undefined : <p className="vl-editor-notice">{t(key)}</p>}
    </div>
  );
}

function BoundEditor({ session, seed, user, resolvePage, onOpenPage }: BoundProperties) {
  const t = useT();
  const live = useLive(session.live);
  const editable = live.status !== "revoked" && live.canWrite;
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
  useEffect(() => {
    // Tiptap would emit an update for the change; leave the document alone.
    if (editor.isEditable !== editable) {
      editor.setEditable(editable, false);
    }
  }, [editor, editable]);

  return (
    <div className="vl-editor">
      <Toolbar editor={editor} />
      <LiveNotice state={live} />
      <EditorContent editor={editor} />
    </div>
  );
}

/**
 * The page editor: Tiptap bound to the page's Yjs document. Opening the
 * document is an effect so React's development double-mount never leaks
 * a session.
 */
export function PageEditor({ connector, pageId, spaceId, ...rest }: PageEditorProperties) {
  const t = useT();
  const [open, setOpen] = useState<{ pageId: string; session: CollabSession }>();
  useEffect(() => {
    const session = connector.open(pageId, spaceId);
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
  }, [connector, pageId, spaceId]);
  if (open?.pageId !== pageId) {
    return (
      <p className="vl-muted" role="status">
        {t("editor-loading")}
      </p>
    );
  }
  return <BoundEditor key={pageId} session={open.session} {...rest} />;
}
