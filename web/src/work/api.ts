// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * My Work tasks (`/api/v1/my-work`, `/api/v1/tasks`) and Work issues with
 * their comments (`/api/v1/work`), over the app's HTTP client so writes
 * carry the CSRF token. Types come from the server's generated client.
 */

import type {
  CommentOut,
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
  ThreadsOut,
  ViewIn,
  ViewOut,
  ViewsOut,
} from "../api/api.gen";
import type { Http } from "../api/http";

/** A new issue. */
interface NewIssue {
  readonly team: string;
  readonly title: string;
  readonly description?: string;
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
export interface WorkApi {
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
}

const WORK = "/api/v1/work";
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
    issue: (key) => http.get<IssueOut>(issuePath(key)),
    async createIssue(issue) {
      const created = await http.send<CreatedIssue>("POST", `${WORK}/issues`, issue);
      return created.issue;
    },
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
