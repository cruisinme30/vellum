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
  CyclesOut,
  GroupOut,
  IssueGroups,
  MyIssuesOut,
  IssueOut,
  MyWorkSettingsInput,
  MyWorkSettingsView,
  MyWorkView,
  TaskView,
  TeamOut,
  TeamsOut,
  ThreadOut,
  ThreadsOut,
  ViewIn,
  ViewOut,
  ViewsOut,
} from "../api/api.gen";
import type { Http } from "../api/http";

import type { TeamFile } from "./settings/team-file";

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

/** How to list issues: the settings a view saves. */
export interface IssueQuery {
  readonly team?: string;
  /** A Work filter tree as JSON text; absent for every issue. */
  readonly filter?: string;
  readonly group_by?: string;
  readonly order_by?: string;
  readonly show_closed?: string;
  readonly layout?: string;
}

/** A triage decision on an issue in a team's inbox. */
export type TriageDecision =
  | { readonly action: "accept"; readonly state?: string }
  | { readonly action: "decline"; readonly reason?: string }
  | { readonly action: "duplicate"; readonly duplicate_of: string }
  | { readonly action: "snooze"; readonly until: string };

/** Which of my issues: assigned (in focus order), created, subscribed or recent activity. */
export type MyIssuesTab = "assigned" | "created" | "subscribed" | "activity";

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
  /** Issues whose title contains `query`, or the issue with that key, at most 8. */
  searchIssues(query: string): Promise<IssueOut[]>;
  issue(key: string): Promise<IssueOut>;
  createIssue(issue: NewIssue): Promise<IssueOut>;
  /** Files an issue and says which chip to write where (from a page selection). */
  fileIssue(issue: NewIssue): Promise<CreatedIssue>;
  setState(key: string, state: string): Promise<IssueOut>;
  /** Changes an issue's priority (0 none, 1 urgent … 4 low). */
  setPriority(key: string, priority: number): Promise<IssueOut>;
  /** An issue's comments, oldest first, across its threads. */
  comments(key: string): Promise<CommentOut[]>;
  addComment(key: string, body: string): Promise<void>;
  /**
   * Links the Slack account behind a code from Slack's "Save as task"
   * reply to the signed-in person (#163). Absent in demo mode.
   */
  linkSlack?(code: string): Promise<void>;
  /** Issues in groups (columns on a board), as a view lays them out. */
  queryIssues(query: IssueQuery): Promise<GroupOut[]>;
  /** A team's triage inbox, oldest first, snoozed issues left out. */
  inbox(team: string): Promise<IssueOut[]>;
  triage(key: string, decision: TriageDecision): Promise<IssueOut>;
  /** A team's cycles with burn-up points, and its velocity. */
  cycles(team: string): Promise<CyclesOut>;
  myIssues(tab: MyIssuesTab): Promise<MyIssuesOut>;
  /** The saved views the person can open, for one team or all. */
  views(team?: string): Promise<ViewOut[]>;
  createView(view: ViewIn): Promise<ViewOut>;
  updateView(id: string, view: ViewIn): Promise<ViewOut>;
  deleteView(id: string): Promise<void>;
  /** A team's whole settings file, as JSON. */
  teamSettings(team: string): Promise<TeamFile>;
  /** Replaces a team's settings file (admins; needs a recent sign-in). */
  saveTeamSettings(team: string, settings: TeamFile): Promise<TeamOut>;
  /** Creates a team (admins). */
  createTeam(team: NewTeam): Promise<TeamOut>;
}

/** A new team. */
export interface NewTeam {
  readonly key: string;
  readonly name: string;
  readonly lead?: string;
  readonly members?: readonly string[];
}

const WORK = "/api/v1/work";
const ISSUE_KEY = /^[a-z][a-z0-9]{1,9}-[1-9]\d*$/i;
const segment = encodeURIComponent;
const issuePath = (key: string) => `${WORK}/issues/${segment(key)}`;

/** Work over a Vellum server. */
export function createHttpWorkApi(http: Http): WorkApi {
  return {
    myWork: () => http.get<MyWorkView>("/api/v1/my-work"),
    async linkSlack(code) {
      await http.send("POST", "/api/v1/my-work/slack-link", { code });
    },
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
    async searchIssues(query) {
      if (ISSUE_KEY.test(query)) {
        try {
          return [await http.get<IssueOut>(issuePath(query.toUpperCase()))];
        } catch {
          // Not a key after all: search titles.
        }
      }
      const found = await http.get<IssueGroups>(`${WORK}/issues`, {
        filter: JSON.stringify({ field: "title", op: "contains", values: [query] }),
        group_by: "none",
        limit: 8,
      });
      return found.groups.flatMap((group) => group.issues).slice(0, 8);
    },
    issue: (key) => http.get<IssueOut>(issuePath(key)),
    async createIssue(issue) {
      const created = await http.send<CreatedIssue>("POST", `${WORK}/issues`, issueBody(issue));
      return created.issue;
    },
    fileIssue: (issue) => http.send<CreatedIssue>("POST", `${WORK}/issues`, issueBody(issue)),
    setState: (key, state) => http.send<IssueOut>("PATCH", issuePath(key), { state }),
    setPriority: (key, priority) => http.send<IssueOut>("PATCH", issuePath(key), { priority }),
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
    async queryIssues(query) {
      const found = await http.get<IssueGroups>(`${WORK}/issues`, { ...query });
      return found.groups;
    },
    async inbox(team) {
      const found = await http.get<IssueGroups>(`${WORK}/teams/${segment(team)}/inbox`);
      return found.groups.flatMap((group) => group.issues);
    },
    triage: (key, decision) => http.send<IssueOut>("POST", `${issuePath(key)}/triage`, decision),
    cycles: (team) => http.get<CyclesOut>(`${WORK}/teams/${segment(team)}/cycles`),
    myIssues: (tab) => http.get<MyIssuesOut>(`${WORK}/me/issues`, { tab }),
    async views(team) {
      const found = await http.get<ViewsOut>(
        `${WORK}/views`,
        team === undefined ? undefined : { team },
      );
      return found.views;
    },
    createView: (view) => http.send<ViewOut>("POST", `${WORK}/views`, view),
    updateView: (id, view) => http.send<ViewOut>("PUT", `${WORK}/views/${segment(id)}`, view),
    async deleteView(id) {
      await http.send("DELETE", `${WORK}/views/${segment(id)}`);
    },
    async teamSettings(team) {
      const found = await http.get<{ settings: TeamFile }>(
        `${WORK}/teams/${segment(team)}/settings`,
      );
      return found.settings;
    },
    // JSON is YAML, so the edited file goes back as the team's YAML text.
    saveTeamSettings: (team, settings) =>
      http.send<TeamOut>("PUT", `${WORK}/teams/${segment(team)}`, {
        yaml: JSON.stringify(settings),
      }),
    createTeam: (team) => http.send<TeamOut>("POST", `${WORK}/teams`, team),
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

/** What an issue chip shows: the title, the state's name and whether it is finished. */
export interface IssueChipStatus {
  readonly title: string;
  readonly state: string;
  readonly closed: boolean;
}

/**
 * Looks up issues for live chips (#93), naming each state the way its team
 * does; the team list is fetched once.
 */
export function issueChipStatus(api: WorkApi): (key: string) => Promise<IssueChipStatus> {
  let teams: Promise<TeamOut[]> | undefined;
  return async (key) => {
    teams ??= api.teams().catch(() => []);
    const [issue, known] = await Promise.all([api.issue(key), teams]);
    const team = known.find((candidate) => candidate.key === issue.team);
    const state = team?.statuses.find((status) => status.key === issue.state)?.name ?? issue.state;
    return {
      title: issue.title,
      state,
      closed: issue.category === "completed" || issue.category === "canceled",
    };
  };
}

export type {
  CommentOut,
  CycleOut,
  CyclesOut,
  FocusedIssue,
  GroupOut,
  IssueOut,
  SlaOut,
  ViewIn,
  ViewOut,
  MyWorkSettingsView,
  MyWorkView,
  TaskView,
  TeamOut,
} from "../api/api.gen";
