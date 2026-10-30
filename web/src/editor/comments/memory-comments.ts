// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { ThreadOut } from "../../api/api.gen";
import type { CommentsApi } from "../../work/api";

/**
 * Comment threads in memory, with the server's rules that the panel relies
 * on: a document ID lists its block threads too, a reply joins its thread,
 * and a reaction is one per person and emoji. For tests and stories.
 */
export function createMemoryCommentsApi(
  author: string,
  clock: () => Date = () => new Date(),
): CommentsApi & { readonly all: () => readonly ThreadOut[] } {
  const threads: ThreadOut[] = [];
  let next = 1;
  const freshId = () => `c${String(next++).padStart(4, "0")}`;
  const now = () => clock().toISOString();
  const byComment = (id: string) => {
    const thread = threads.find((t) => t.comments.some((c) => c.id === id));
    if (!thread) return Promise.reject(new Error(`unknown comment ${id}`));
    return Promise.resolve(thread);
  };
  const byThread = (id: string) => {
    const thread = threads.find((t) => t.id === id);
    if (!thread) return Promise.reject(new Error(`unknown thread ${id}`));
    return Promise.resolve(thread);
  };
  const replace = (thread: ThreadOut) => {
    threads.splice(
      threads.findIndex((t) => t.id === thread.id),
      1,
      thread,
    );
    return thread;
  };
  return {
    all: () => threads,
    threads(target) {
      const [document, block] = target.split("#");
      return Promise.resolve(
        threads.filter(
          (t) =>
            (t.target.doc === document || t.target.issue === document) &&
            (block === undefined || t.target.block === block),
        ),
      );
    },
    startThread(input) {
      const [document = "", block] = input.target.split("#");
      const id = freshId();
      const thread: ThreadOut = {
        id,
        target: /^[A-Z][A-Z0-9]*-\d+$/.test(document)
          ? { issue: document }
          : { doc: document, ...(block === undefined ? {} : { block }) },
        ...(input.quote === undefined ? {} : { quote: input.quote }),
        ...(input.assignTo === undefined ? {} : { assignee: input.assignTo }),
        comments: [
          {
            id,
            author,
            body: input.body,
            by_model: false,
            created_at: now(),
            deleted: false,
            reactions: [],
          },
        ],
      };
      threads.push(thread);
      return Promise.resolve(thread);
    },
    async reply(commentId, body) {
      const thread = await byComment(commentId);
      return replace({
        ...thread,
        comments: [
          ...thread.comments,
          {
            id: freshId(),
            author,
            body,
            by_model: false,
            created_at: now(),
            deleted: false,
            reactions: [],
          },
        ],
      });
    },
    async editComment(commentId, body) {
      const thread = await byComment(commentId);
      return replace({
        ...thread,
        comments: thread.comments.map((c) =>
          c.id === commentId ? { ...c, body, edited_at: now() } : c,
        ),
      });
    },
    async deleteComment(commentId) {
      const thread = await byComment(commentId);
      return replace({
        ...thread,
        comments: thread.comments.map((c) =>
          c.id === commentId ? { ...c, body: "", deleted: true } : c,
        ),
      });
    },
    async react(commentId, emoji, on) {
      const thread = await byComment(commentId);
      return replace({
        ...thread,
        comments: thread.comments.map((c) => {
          if (c.id !== commentId) return c;
          const others = c.reactions.filter((r) => r.emoji !== emoji);
          const by = new Set(c.reactions.find((r) => r.emoji === emoji)?.by);
          if (on) by.add(author);
          else by.delete(author);
          return {
            ...c,
            reactions:
              by.size === 0 ? others : [...others, { emoji, by: [...by] }],
          };
        }),
      });
    },
    async resolve(threadId, resolved) {
      const thread = await byThread(threadId);
      const next = { ...thread };
      delete next.resolved_at;
      delete next.resolved_by;
      return replace(
        resolved ? { ...next, resolved_at: now(), resolved_by: author } : next,
      );
    },
    async assign(threadId, assignee) {
      const thread = await byThread(threadId);
      const next = { ...thread };
      delete next.assignee;
      return replace(assignee === undefined ? next : { ...next, assignee });
    },
  };
}
