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
  IssueGroups,
  IssueOut,
  MyWorkSettingsInput,
  MyWorkSettingsView,
  MyWorkView,
  TaskView,
  TeamOut,
  TeamsOut,
  ThreadsOut,
} from "../api/api.gen";
import type { Http } from "../api/http";

/** A new issue. */
interface NewIssue {
  readonly team: string;
  readonly title: string;
  readonly description?: string;
}

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
  /** Issues whose title contains `query`, or the issue with that key, at most 8. */
  searchIssues(query: string): Promise<IssueOut[]>;
  issue(key: string): Promise<IssueOut>;
  createIssue(issue: NewIssue): Promise<IssueOut>;
  setState(key: string, state: string): Promise<IssueOut>;
  /** An issue's comments, oldest first, across its threads. */
  comments(key: string): Promise<CommentOut[]>;
  addComment(key: string, body: string): Promise<void>;
}

const WORK = "/api/v1/work";
const ISSUE_KEY = /^[a-z][a-z0-9]{1,9}-[1-9]\d*$/i;
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
      const created = await http.send<CreatedIssue>("POST", `${WORK}/issues`, issue);
      return created.issue;
    },
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
  IssueOut,
  MyWorkSettingsView,
  MyWorkView,
  TaskView,
  TeamOut,
} from "../api/api.gen";
