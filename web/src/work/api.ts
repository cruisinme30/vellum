// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * My Work tasks (`/api/v1/my-work`, `/api/v1/tasks`) and Work issues with
 * their comments (`/api/v1/work`), over the app's HTTP client so writes
 * carry the CSRF token. Types come from the server's generated client.
 */

import type {
  CommentOut,
  CommentResult,
  CompleteView,
  CreatedIssue,
  IssueGroups,
  IssueOut,
  MyWorkSettingsInput,
  MyWorkSettingsView,
  MyWorkView,
  TaskView,
  TeamOut,
  TeamsOut,
  ThreadOut,
  ThreadsOut,
} from "../api/api.gen";
import type { Http } from "../api/http";

/** A new issue. */
interface NewIssue {
  readonly team: string;
  readonly title: string;
  readonly description?: string;
  /** `<doc id>#blk-…` when filed from a page's selection (R-WK-7). */
  readonly sourceBlock?: string;
  /** The selected text, kept on the issue as a quote. */
  readonly quote?: string;
}

/** A new comment thread on a page, a block or an issue. */
interface NewThread {
  /** An issue key, a document ID or `<doc id>#blk-…`. */
  readonly target: string;
  readonly body: string;
  /** The selected text, for a block thread. */
  readonly quote?: string;
  /** "Assign to" this handle. */
  readonly assignTo?: string;
}

/**
 * Comment threads (#94): the work core owns them, so the editor's comments
 * panel and an issue page both use these calls.
 */
export interface CommentsApi {
  /** The threads on a target; a document ID returns its block threads too. */
  threads(target: string): Promise<ThreadOut[]>;
  startThread(thread: NewThread): Promise<ThreadOut | undefined>;
  reply(commentId: string, body: string): Promise<ThreadOut | undefined>;
  editComment(commentId: string, body: string): Promise<ThreadOut | undefined>;
  deleteComment(commentId: string): Promise<ThreadOut | undefined>;
  /** Adds (`on`) or takes back a reaction. */
  react(commentId: string, emoji: string, on: boolean): Promise<ThreadOut | undefined>;
  resolve(threadId: string, resolved: boolean): Promise<ThreadOut | undefined>;
  /** Assigns the thread to a handle; `undefined` unassigns. */
  assign(threadId: string, assignee: string | undefined): Promise<ThreadOut | undefined>;
}

/** The Work and My Work operations the web app uses. */
export interface WorkApi extends CommentsApi {
  /** The signed-in person's tasks, in buckets. */
  myWork(): Promise<MyWorkView>;
  /** Adds a task from quick-add text such as "Call Sam tomorrow !high". */
  addTask(text: string): Promise<TaskView>;
  /** Ticks or unticks a task. */
  setDone(task: string, done: boolean): Promise<TaskView>;
  /** Takes a mention (a task of kind `mention`) out of My Work. */
  dismissMention(task: string): Promise<void>;
  /** The person's time zone, start of day and digest time. */
  settings(): Promise<MyWorkSettingsView>;
  saveSettings(settings: MyWorkSettingsInput): Promise<MyWorkSettingsView>;
  teams(): Promise<TeamOut[]>;
  /** A team's open issues, most important first. */
  issues(team: string): Promise<IssueOut[]>;
  issue(key: string): Promise<IssueOut>;
  createIssue(issue: NewIssue): Promise<IssueOut>;
  /** Files an issue and says which chip to write where (from a page selection). */
  fileIssue(issue: NewIssue): Promise<CreatedIssue>;
  setState(key: string, state: string): Promise<IssueOut>;
  /** An issue's comments, oldest first, across its threads. */
  comments(key: string): Promise<CommentOut[]>;
  addComment(key: string, body: string): Promise<void>;
}

const WORK = "/api/v1/work";
const segment = encodeURIComponent;
const issuePath = (key: string) => `${WORK}/issues/${segment(key)}`;

/** Work over a Vellum server. */
export function createHttpWorkApi(http: Http): WorkApi {
  return {
    myWork: () => http.get<MyWorkView>("/api/v1/my-work"),
    addTask: (text) => http.send<TaskView>("POST", "/api/v1/tasks", { text }),
    async setDone(task, done) {
      const path = `/api/v1/tasks/${segment(task)}`;
      if (!done) return http.send<TaskView>("POST", `${path}/reopen`, {});
      const completed = await http.send<CompleteView>("POST", `${path}/complete`, {});
      return completed.task;
    },
    async dismissMention(task) {
      const comment = task.replace(/^mention\./, "");
      await http.send("POST", `/api/v1/my-work/mentions/${segment(comment)}/dismiss`, {});
    },
    settings: () => http.get<MyWorkSettingsView>("/api/v1/my-work/settings"),
    saveSettings: (settings) =>
      http.send<MyWorkSettingsView>("PUT", "/api/v1/my-work/settings", settings),
    async teams() {
      const list = await http.get<TeamsOut>(`${WORK}/teams`);
      return list.teams;
    },
    async issues(team) {
      const found = await http.get<IssueGroups>(`${WORK}/issues`, {
        team,
        group_by: "none",
        order_by: "priority",
        show_closed: "none",
      });
      return found.groups.flatMap((group) => group.issues);
    },
    issue: (key) => http.get<IssueOut>(issuePath(key)),
    async createIssue(issue) {
      const created = await http.send<CreatedIssue>("POST", `${WORK}/issues`, issueBody(issue));
      return created.issue;
    },
    fileIssue: (issue) => http.send<CreatedIssue>("POST", `${WORK}/issues`, issueBody(issue)),
    setState: (key, state) => http.send<IssueOut>("PATCH", issuePath(key), { state }),
    async comments(key) {
      const list = await http.get<ThreadsOut>(`${WORK}/comments`, { target: key });
      return list.threads
        .flatMap((thread) => thread.comments)
        .filter((comment) => !comment.deleted)
        .toSorted((a, b) => a.created_at.localeCompare(b.created_at));
    },
    async addComment(key, body) {
      await http.send("POST", `${WORK}/comments`, { target: key, body });
    },
    async threads(target) {
      const list = await http.get<ThreadsOut>(`${WORK}/comments`, { target });
      return list.threads;
    },
    startThread: (thread) =>
      threadOf(
        http.send<CommentResult>("POST", `${WORK}/comments`, {
          target: thread.target,
          body: thread.body,
          quote: thread.quote,
          assign_to: thread.assignTo,
        }),
      ),
    reply: (commentId, body) =>
      threadOf(http.send<CommentResult>("POST", `${WORK}/comments`, { reply_to: commentId, body })),
    editComment: (commentId, body) =>
      threadOf(http.send<CommentResult>("PATCH", commentPath(commentId), { body })),
    deleteComment: (commentId) =>
      threadOf(http.send<CommentResult>("DELETE", commentPath(commentId))),
    react: (commentId, emoji, on) =>
      threadOf(
        http.send<CommentResult>("POST", `${commentPath(commentId)}/reactions`, { emoji, on }),
      ),
    resolve: (threadId, resolved) =>
      threadOf(http.send<CommentResult>("POST", `${threadPath(threadId)}/resolve`, { resolved })),
    assign: (threadId, assignee) =>
      threadOf(http.send<CommentResult>("POST", `${threadPath(threadId)}/assign`, { assignee })),
  };
}

const commentPath = (id: string) => `${WORK}/comments/${segment(id)}`;
const threadPath = (id: string) => `${WORK}/threads/${segment(id)}`;

async function threadOf(result: Promise<CommentResult>): Promise<ThreadOut | undefined> {
  const { thread } = await result;
  return thread ?? undefined;
}

function issueBody(issue: NewIssue) {
  return {
    team: issue.team,
    title: issue.title,
    description: issue.description,
    source_block: issue.sourceBlock,
    quote: issue.quote,
  };
}

export type {
  CommentOut,
  IssueOut,
  MyWorkSettingsView,
  MyWorkView,
  TaskView,
  TeamOut,
} from "../api/api.gen";
