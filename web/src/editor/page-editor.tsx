// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { getSchema, type JSONContent } from "@tiptap/core";
import { Collaboration } from "@tiptap/extension-collaboration";
import { CollaborationCaret } from "@tiptap/extension-collaboration-caret";
import { EditorContent, useEditor } from "@tiptap/react";
import { useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from "react";
import * as Y from "yjs";

import { useT } from "../i18n/i18n";

import type { ChipSources } from "./chips/items";
import { PresenceBar } from "./collab/presence";
import type {
  CollabConnector,
  CollabSession,
  CollabUser,
  LiveConnection,
  LiveState,
} from "./collab/provider";
import { createSaver, type SaveStatus, type Saver } from "./collab/saving";
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
  /** How long after the last change a version is written; 30 s by default. */
  readonly idleSaveMs?: number;
  /** Extra controls in the editor's bar, before Done. */
  readonly actions?: ReactNode;
  /** Told how to save the page now and whether it is editable; `undefined` once closed. */
  readonly onControls?: (controls?: EditorControls) => void;
  /**
   * People, pages and issues for the `@` and `[[` menus and live issue
   * chips. Read when the page opens: pass one object for the page's life.
   */
  readonly chips?: ChipSources | undefined;
}

/** What the page around the editor may do with it. */
export interface EditorControls {
  /** Writes a version now; resolves `false` when that failed. */
  readonly save: () => Promise<boolean>;
  /** Whether this person may change the page. */
  readonly canWrite: boolean;
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

const SAVE_LABELS: Record<SaveStatus, string> = {
  saved: "editor-saved",
  unsaved: "editor-unsaved",
  saving: "editor-saving",
  failed: "editor-save-failed",
};

/** Whether a key press is the save shortcut (Ctrl+S, or Cmd+S on a Mac). */
const isSaveKey = (event: KeyboardEvent) =>
  (event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === "s";

/**
 * Where this editor's changes stand, and Done, which writes a version now.
 * Versions are also written on their own after a quiet spell (ADR 0010).
 */
function SaveControls({
  saver,
  canSave,
  children,
}: {
  readonly saver: Saver;
  readonly canSave: boolean;
  readonly children?: ReactNode;
}) {
  const t = useT();
  const status = useSyncExternalStore(saver.subscribe, saver.getSnapshot);
  return (
    <div className="vl-editor-save">
      <span className="vl-save-status" aria-live="polite" data-status={status}>
        {canSave ? t(SAVE_LABELS[status]) : ""}
      </span>
      {children}
      <button
        type="button"
        className="vl-filled-button vl-done-button"
        disabled={!canSave || status === "saving"}
        title={t("editor-done-hint")}
        aria-keyshortcuts="Control+S Meta+S"
        onClick={() => {
          void saver.save();
        }}
      >
        {t("editor-done")}
      </button>
    </div>
  );
}

const LOCAL_LIVE = (): LiveConnection => LOCAL;

function BoundEditor({
  session,
  seed,
  user,
  resolvePage,
  onOpenPage,
  idleSaveMs,
  actions,
  onControls,
  chips,
}: BoundProperties) {
  const t = useT();
  const live = useLive(session.live);
  const [saver] = useState(() =>
    createSaver(session.doc, session.live ?? LOCAL_LIVE(), {
      ...(idleSaveMs === undefined ? {} : { idleMs: idleSaveMs }),
    }),
  );
  useEffect(
    () => () => {
      saver.dispose();
    },
    [saver],
  );
  const editable = live.status !== "revoked" && live.canWrite;
  useEffect(() => {
    onControls?.({
      save: async () => {
        await saver.save();
        return saver.getSnapshot() !== "failed";
      },
      canWrite: editable,
    });
  }, [onControls, saver, editable]);
  useEffect(
    () => () => {
      onControls?.();
    },
    [onControls],
  );
  const base = useMemo(
    () => vellumExtensions({ t, resolvePage, history: false, chips }),
    [t, resolvePage, chips],
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
        handleKeyDown: (_view, event) => {
          if (!isSaveKey(event)) {
            return false;
          }
          event.preventDefault();
          void saver.save();
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
      <div className="vl-editor-bar">
        <Toolbar editor={editor} />
        <SaveControls saver={saver} canSave={editable && live.status === "online"}>
          <PresenceBar awareness={session.awareness} live={session.live ? live : undefined} />
          {actions}
        </SaveControls>
      </div>
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
