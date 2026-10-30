// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { getSchema, type JSONContent } from "@tiptap/core";
import { Collaboration } from "@tiptap/extension-collaboration";
import { CollaborationCaret } from "@tiptap/extension-collaboration-caret";
import { type Editor, EditorContent, useEditor } from "@tiptap/react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import * as Y from "yjs";

import { useT } from "../i18n/i18n";
import { Icon } from "../shell/icon";
import type { WorkApi } from "../work/api";

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
import { CommentMarkers, ensureAnchor, type Anchor } from "./comments/anchor";
import { CommentsPanel } from "./comments/comments-panel";
import { FileIssue } from "./comments/file-issue";
import { vellumExtensions } from "./extensions";
import type { ResolvePage } from "./nodes/inline";
import { isSelectionTaskKey, selectionTaskText, useReadOnlyTaskKey } from "./selection-task";
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
   * Comments and issues, from a Vellum server; without it (demo mode) the
   * Comment and Create issue controls are absent.
   */
  readonly work?: WorkApi | undefined;
  /** The signed-in person's handle, to mark their own reactions. */
  readonly me?: string | undefined;
  /**
   * People, pages and issues for the `@` and `[[` menus and live issue
   * chips. Read when the page opens: pass one object for the page's life.
   */
  readonly chips?: ChipSources | undefined;
  /**
   * Turns the selected text into a task (R-MW-3): Ctrl/Cmd+Alt+Enter or the
   * toolbar button while editing, a bare `T` on a read-only page.
   */
  readonly onSelectionTask?: ((text: string) => void) | undefined;
}

/** What the page around the editor may do with it. */
export interface EditorControls {
  /** Writes a version now; resolves `false` when that failed. */
  readonly save: () => Promise<boolean>;
  /** Whether this person may change the page. */
  readonly canWrite: boolean;
}

interface BoundProperties extends Omit<PageEditorProperties, "connector" | "spaceId"> {
  readonly session: CollabSession;
}

/** Which side tool is open, and the selection it was opened on. */
type Side =
  | { readonly kind: "comments"; readonly anchor: Anchor }
  | { readonly kind: "issue"; readonly anchor: Anchor };

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

/** The Comment and Create issue buttons, with the issue form when open. */
function WorkActions({
  work,
  editable,
  side,
  onToggle,
  onClose,
  pageId,
  editor,
}: {
  readonly work: WorkApi;
  readonly editable: boolean;
  readonly side: Side | undefined;
  readonly onToggle: (kind: Side["kind"]) => void;
  readonly onClose: () => void;
  readonly pageId: string;
  readonly editor: Editor;
}) {
  const t = useT();
  return (
    <>
      <button
        type="button"
        className="vl-icon-button"
        aria-label={t("comments-open")}
        title={t("comments-open")}
        aria-expanded={side?.kind === "comments"}
        onClick={() => {
          onToggle("comments");
        }}
      >
        <Icon name="comment" />
      </button>
      {editable && (
        <span className="vl-file-issue-anchor">
          <button
            type="button"
            className="vl-icon-button"
            aria-label={t("file-issue-open")}
            title={t("file-issue-open")}
            aria-expanded={side?.kind === "issue"}
            onClick={() => {
              onToggle("issue");
            }}
          >
            <Icon name="add_task" />
          </button>
          {side?.kind === "issue" && (
            <FileIssue
              api={work}
              documentId={pageId}
              editor={editor}
              anchor={side.anchor}
              onClose={onClose}
            />
          )}
        </span>
      )}
    </>
  );
}

function BoundEditor({
  pageId,
  session,
  seed,
  user,
  resolvePage,
  onOpenPage,
  idleSaveMs,
  actions,
  onControls,
  work,
  me,
  chips,
  onSelectionTask,
}: BoundProperties) {
  const t = useT();
  // The editor is built once per session; the ref sees the latest callback.
  const selectionTask = useRef(onSelectionTask);
  useEffect(() => {
    selectionTask.current = onSelectionTask;
  }, [onSelectionTask]);
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
        CommentMarkers,
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
        handleKeyDown: (view, event) => {
          const toTask = selectionTask.current;
          if (toTask !== undefined && isSelectionTaskKey(event)) {
            const selected = selectionTaskText(view.state);
            if (selected === undefined) return false;
            event.preventDefault();
            toTask(selected);
            return true;
          }
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
  useReadOnlyTaskKey(editor, editable, onSelectionTask);
  useEffect(() => {
    // Tiptap would emit an update for the change; leave the document alone.
    if (editor.isEditable !== editable) {
      editor.setEditable(editable, false);
    }
  }, [editor, editable]);

  const [side, setSide] = useState<Side>();
  const closeSide = useCallback(() => {
    setSide(undefined);
  }, []);
  const openSide = (kind: Side["kind"]) => {
    setSide(side?.kind === kind ? undefined : { kind, anchor: ensureAnchor(editor) });
  };
  const workActions = work && (
    <WorkActions
      work={work}
      editable={editable}
      side={side}
      onToggle={openSide}
      onClose={closeSide}
      pageId={pageId}
      editor={editor}
    />
  );

  return (
    <div className="vl-editor">
      <div className="vl-editor-bar">
        <Toolbar
          editor={editor}
          onSelectionTask={
            onSelectionTask === undefined || !editable
              ? undefined
              : () => {
                  const selected = selectionTaskText(editor.state);
                  if (selected !== undefined) onSelectionTask(selected);
                }
          }
        />
        <SaveControls saver={saver} canSave={editable && live.status === "online"}>
          {workActions}
          <PresenceBar awareness={session.awareness} live={session.live ? live : undefined} />
          {actions}
        </SaveControls>
      </div>
      <LiveNotice state={live} />
      <EditorContent editor={editor} />
      {work && side?.kind === "comments" && (
        <CommentsPanel
          api={work}
          documentId={pageId}
          editor={editor}
          me={me}
          anchor={side.anchor}
          onClose={closeSide}
        />
      )}
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
  return <BoundEditor key={pageId} pageId={pageId} session={open.session} {...rest} />;
}
