// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Editor } from "@tiptap/core";
import { TextSelection } from "@tiptap/pm/state";
import { useEffect, useId, useMemo, useState } from "react";

import type { ThreadOut } from "../../api/api.gen";
import { failureText } from "../../data/failure";
import { useT, type Translate } from "../../i18n/i18n";
import { CopyDiagnostic } from "../../shell/copy-diagnostic";
import { Icon } from "../../shell/icon";
import type { CommentsApi } from "../../work/api";

import { blockPosition, setCommentMarkers, targetOf, type Anchor } from "./anchor";
import { field, submitWith, Thread } from "./thread";

/** Props of {@link CommentsPanel}. */
export interface CommentsPanelProperties {
  readonly api: CommentsApi;
  readonly documentId: string;
  readonly editor: Editor;
  /** The signed-in person's handle, to mark their reactions. */
  readonly me: string | undefined;
  /** Where a new thread goes; taken from the selection when the panel opened. */
  readonly anchor: Anchor;
  readonly onClose: () => void;
}

type Filter = "open" | "resolved";

/** Replaces a thread in the list, or adds it at the end. */
function upsert(threads: readonly ThreadOut[], thread: ThreadOut): ThreadOut[] {
  const index = threads.findIndex((t) => t.id === thread.id);
  return index === -1 ? [...threads, thread] : threads.with(index, thread);
}

/** Selects the block a thread is on and scrolls it into view. */
function reveal(editor: Editor, blockId: string) {
  const pos = blockPosition(editor.state.doc, blockId);
  if (pos === undefined) return;
  const inside = Math.min(pos + 1, editor.state.doc.content.size);
  editor.view.dispatch(
    editor.state.tr.setSelection(TextSelection.near(editor.state.doc.resolve(inside))),
  );
  editor.view.focus();
  const dom = editor.view.nodeDOM(pos);
  if (dom instanceof HTMLElement) dom.scrollIntoView({ block: "center", behavior: "smooth" });
}

/**
 * The editor's comments panel (#94): threads on the page and its blocks,
 * with replies, reactions, resolve and assign. The work core owns the
 * threads (ADR 0010, D232); the panel only calls its API and marks the
 * blocks that have open threads.
 */
export function CommentsPanel({
  api,
  documentId,
  editor,
  me,
  anchor,
  onClose,
}: CommentsPanelProperties) {
  const t = useT();
  const headingId = useId();
  const [threads, setThreads] = useState<ThreadOut[]>();
  const [loadError, setLoadError] = useState<string>();
  const [error, setError] = useState<string>();
  const [filter, setFilter] = useState<Filter>("open");
  const [active, setActive] = useState<string>();

  useEffect(() => {
    let current = true;
    api.threads(documentId).then(
      (found) => {
        if (current) setThreads(found);
      },
      (error_: unknown) => {
        if (current) setLoadError(failureText(error_));
      },
    );
    return () => {
      current = false;
    };
  }, [api, documentId]);

  const openBlocks = useMemo(
    () =>
      new Set(
        (threads ?? [])
          .filter((thread) => thread.resolved_at == undefined)
          .map((thread) => thread.target.block)
          .filter((block): block is string => typeof block === "string"),
      ),
    [threads],
  );
  useEffect(() => {
    setCommentMarkers(editor, openBlocks, active);
  }, [editor, openBlocks, active]);
  useEffect(
    () => () => {
      setCommentMarkers(editor, new Set());
    },
    [editor],
  );

  const run = (call: Promise<ThreadOut | undefined>) => {
    setError(undefined);
    call.then(
      (thread) => {
        if (thread) setThreads((list) => upsert(list ?? [], thread));
      },
      (error_: unknown) => {
        setError(failureText(error_));
      },
    );
    return call;
  };

  const shown = (threads ?? []).filter((thread) =>
    filter === "open" ? thread.resolved_at == undefined : thread.resolved_at != undefined,
  );

  return (
    <aside className="vl-comments" aria-labelledby={headingId}>
      <header className="vl-comments-head">
        <h2 id={headingId}>{t("comments-title")}</h2>
        <button
          type="button"
          className="vl-icon-button"
          aria-label={t("comments-close")}
          title={t("comments-close")}
          onClick={onClose}
        >
          <Icon name="close" />
        </button>
      </header>
      <NewThread
        anchor={anchor}
        onSubmit={(body, assignTo) =>
          run(
            api.startThread({
              target: targetOf(documentId, anchor.blockId),
              body,
              ...(anchor.quote === "" ? {} : { quote: anchor.quote }),
              ...(assignTo === "" ? {} : { assignTo }),
            }),
          )
        }
      />
      {error !== undefined && (
        <p className="vl-field-error" role="alert">
          {error}
          <CopyDiagnostic message={error} />
        </p>
      )}
      <div className="vl-segmented" role="group" aria-label={t("comments-filter")}>
        {(["open", "resolved"] as const).map((value) => (
          <button
            key={value}
            type="button"
            aria-pressed={filter === value}
            onClick={() => {
              setFilter(value);
            }}
          >
            {t(`comments-filter-${value}`)}
          </button>
        ))}
      </div>
      <ThreadsState threads={threads} loadError={loadError} shown={shown.length} filter={filter} />
      {shown.length > 0 && (
        <ol className="vl-thread-list">
          {shown.map((thread) => (
            <Thread
              key={thread.id}
              thread={thread}
              me={me}
              active={active === thread.target.block}
              onReveal={(block) => {
                setActive(block);
                reveal(editor, block);
              }}
              onReply={(commentId, body) => run(api.reply(commentId, body))}
              onReact={(commentId, emoji, on) => run(api.react(commentId, emoji, on))}
              onResolve={(resolved) => run(api.resolve(thread.id, resolved))}
              onAssign={(assignee) => run(api.assign(thread.id, assignee))}
            />
          ))}
        </ol>
      )}
    </aside>
  );
}

/** Loading, a failed load, or an empty list; nothing once threads show. */
function ThreadsState({
  threads,
  loadError,
  shown,
  filter,
}: {
  readonly threads: readonly ThreadOut[] | undefined;
  readonly loadError: string | undefined;
  readonly shown: number;
  readonly filter: Filter;
}) {
  const t = useT();
  if (loadError !== undefined) {
    return (
      <p className="vl-field-error" role="alert">
        {loadError}
        <CopyDiagnostic message={loadError} />
      </p>
    );
  }
  if (threads === undefined) {
    return (
      <p className="vl-muted" role="status">
        {t("loading")}
      </p>
    );
  }
  return shown === 0 ? <p className="vl-muted">{t(`comments-none-${filter}`)}</p> : undefined;
}

/** What a new thread is on, in words. */
function anchorText(t: Translate, anchor: Anchor) {
  if (anchor.quote !== "") return t("comments-on-quote", { quote: anchor.quote });
  return t(anchor.blockId === undefined ? "comments-on-page" : "comments-on-block");
}

function NewThread({
  anchor,
  onSubmit,
}: {
  readonly anchor: Anchor;
  readonly onSubmit: (body: string, assignTo: string) => Promise<unknown>;
}) {
  const t = useT();
  return (
    <form
      className="vl-comment-form"
      onSubmit={submitWith((form) => {
        const body = field(form, "body");
        return body === "" ? undefined : onSubmit(body, field(form, "assign").replace(/^@/, ""));
      })}
    >
      <p className="vl-muted vl-comment-anchor">{anchorText(t, anchor)}</p>
      <label>
        {t("comments-new")}
        <textarea name="body" rows={3} required />
      </label>
      <label>
        {t("comments-assign-to")}
        <input name="assign" placeholder="@handle" autoComplete="off" />
      </label>
      <button type="submit" className="vl-filled-button">
        {t("comments-send")}
      </button>
    </form>
  );
}
