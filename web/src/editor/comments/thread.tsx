// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { ComponentProps } from "react";

import type { CommentOut, ThreadOut } from "../../api/api.gen";
import { useT } from "../../i18n/i18n";

/** Quick reactions offered on every comment. */
const QUICK_REACTIONS = ["👍", "🎉", "❤️", "👀"] as const;

const TIME = new Intl.DateTimeFormat("en", {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
});

type Submit = NonNullable<ComponentProps<"form">["onSubmit"]>;

/** Reads a form field as trimmed text. */
export const field = (form: HTMLFormElement, name: string) => {
  const value = new FormData(form).get(name);
  return typeof value === "string" ? value.trim() : "";
};

/** Handles a form whose action resolves on success, resetting it then. */
export function submitWith(action: (form: HTMLFormElement) => Promise<unknown> | undefined) {
  const submit: Submit = (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    void action(form)?.then(
      () => {
        form.reset();
      },
      () => {
        // The panel shows the failure; keep what was typed.
      },
    );
  };
  return submit;
}

interface ThreadProperties {
  readonly thread: ThreadOut;
  readonly me: string | undefined;
  readonly active: boolean;
  readonly onReveal: (block: string) => void;
  readonly onReply: (commentId: string, body: string) => Promise<unknown>;
  readonly onReact: (commentId: string, emoji: string, on: boolean) => Promise<unknown>;
  readonly onResolve: (resolved: boolean) => Promise<unknown>;
  readonly onAssign: (assignee?: string) => Promise<unknown>;
}

export function Thread({
  thread,
  me,
  active,
  onReveal,
  onReply,
  onReact,
  onResolve,
  onAssign,
}: ThreadProperties) {
  const t = useT();
  const resolved = thread.resolved_at != undefined;
  const comments = thread.comments.filter((comment) => !comment.deleted);
  const last = thread.comments.at(-1);
  const block = thread.target.block ?? undefined;
  return (
    <li className={active ? "vl-thread vl-thread-active" : "vl-thread"}>
      {thread.quote != undefined && thread.quote !== "" && (
        <blockquote className="vl-thread-quote">{thread.quote}</blockquote>
      )}
      {block !== undefined && (
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            onReveal(block);
          }}
        >
          {t("comments-show-in-page")}
        </button>
      )}
      <ol className="vl-comment-list">
        {comments.map((comment) => (
          <Comment key={comment.id} comment={comment} me={me} onReact={onReact} />
        ))}
      </ol>
      <ThreadMeta thread={thread} />
      <ThreadActions
        resolved={resolved}
        assigned={thread.assignee != undefined}
        onResolve={onResolve}
        onAssign={onAssign}
      />
      {!resolved && last !== undefined && (
        <form
          className="vl-comment-form vl-reply-form"
          onSubmit={submitWith((form) => {
            const body = field(form, "body");
            return body === "" ? undefined : onReply(last.id, body);
          })}
        >
          <label>
            {t("comments-reply")}
            <textarea name="body" rows={2} required />
          </label>
          <button type="submit" className="vl-text-button">
            {t("comments-send-reply")}
          </button>
        </form>
      )}
    </li>
  );
}

/** Who the thread is assigned to, and who resolved it. */
function ThreadMeta({ thread }: { readonly thread: ThreadOut }) {
  const t = useT();
  const assigned =
    thread.assignee == undefined
      ? t("comments-unassigned")
      : t("comments-assigned", { handle: thread.assignee });
  const resolved =
    thread.resolved_at == undefined
      ? ""
      : ` · ${t("comments-resolved-by", {
          handle: thread.resolved_by ?? "",
          when: TIME.format(new Date(thread.resolved_at)),
        })}`;
  return (
    <p className="vl-muted vl-thread-meta">
      {assigned}
      {resolved}
    </p>
  );
}

function ThreadActions({
  resolved,
  assigned,
  onResolve,
  onAssign,
}: {
  readonly resolved: boolean;
  readonly assigned: boolean;
  readonly onResolve: (resolved: boolean) => Promise<unknown>;
  readonly onAssign: (assignee?: string) => Promise<unknown>;
}) {
  const t = useT();
  return (
    <div className="vl-thread-actions">
      <button
        type="button"
        className="vl-text-button"
        onClick={() => {
          void onResolve(!resolved);
        }}
      >
        {t(resolved ? "comments-reopen" : "comments-resolve")}
      </button>
      {assigned ? (
        <button
          type="button"
          className="vl-text-button"
          onClick={() => {
            void onAssign();
          }}
        >
          {t("comments-unassign")}
        </button>
      ) : (
        <AssignForm onAssign={onAssign} />
      )}
    </div>
  );
}

function AssignForm({ onAssign }: { readonly onAssign: (assignee?: string) => Promise<unknown> }) {
  const t = useT();
  return (
    <form
      className="vl-assign-form"
      onSubmit={submitWith((form) => {
        const handle = field(form, "assignee").replace(/^@/, "");
        return handle === "" ? undefined : onAssign(handle);
      })}
    >
      <label>
        <span className="vl-visually-hidden">{t("comments-assign-thread")}</span>
        <input name="assignee" placeholder="@handle" autoComplete="off" />
      </label>
      <button type="submit" className="vl-text-button">
        {t("comments-assign")}
      </button>
    </form>
  );
}

function Comment({
  comment,
  me,
  onReact,
}: {
  readonly comment: CommentOut;
  readonly me: string | undefined;
  readonly onReact: (commentId: string, emoji: string, on: boolean) => Promise<unknown>;
}) {
  const t = useT();
  const reacted = (emoji: string) =>
    me !== undefined &&
    comment.reactions.some((reaction) => reaction.emoji === emoji && reaction.by.includes(me));
  const counts = new Map(comment.reactions.map((r) => [r.emoji, r.by.length]));
  const emojis = [...new Set([...QUICK_REACTIONS, ...counts.keys()])];
  return (
    <li className="vl-comment">
      <p className="vl-muted">
        {t("issue-comment-by", {
          author: comment.author,
          when: TIME.format(new Date(comment.created_at)),
        })}
        {comment.by_model && (
          <span className="vl-chip vl-model-badge">{t("comments-by-model")}</span>
        )}
      </p>
      <p className="vl-comment-body">{comment.body}</p>
      <div className="vl-reactions" role="group" aria-label={t("comments-reactions")}>
        {emojis.map((emoji) => {
          const count = counts.get(emoji) ?? 0;
          const on = reacted(emoji);
          return (
            <button
              key={emoji}
              type="button"
              className="vl-reaction"
              aria-pressed={on}
              aria-label={t("comments-react", { emoji, count })}
              onClick={() => {
                void onReact(comment.id, emoji, !on);
              }}
            >
              <span aria-hidden="true">{emoji}</span>
              {count > 0 && <span aria-hidden="true">{count}</span>}
            </button>
          );
        })}
      </div>
    </li>
  );
}
