// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** Fakes and fixtures shared by the Work and My Work tests. */

import { render } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import type { ReactNode } from "react";
import { MemoryRouter, Routes } from "react-router";
import { vi } from "vitest";

import { createNorthstarStore } from "../data/mock/northstar";
import { ServerApisProvider } from "../data/server-apis";
import type { WorkspaceStore } from "../data/store";
import { WorkspaceProvider } from "../data/workspace-context";
import type { CollabConnector } from "../editor/collab/provider";
import { createMemoryCommentsApi } from "../editor/comments/memory-comments";
import { I18nProvider } from "../i18n/i18n";

import type { IssueOut, ListView, MyWorkView, ParseView, TaskView, TeamOut, WorkApi } from "./api";

function task(id: string, title: string, extra: Partial<TaskView> = {}): TaskView {
  return {
    id,
    title,
    assignees: [],
    done: false,
    fields: { labels: [], remind: [] },
    kind: "checklist",
    overdue: false,
    subtasks: [],
    ...extra,
  };
}

const TEAM: TeamOut = {
  id: "t1",
  key: "ENG",
  name: "Engineering",
  cycles: false,
  default_status: "todo",
  labels: [],
  members: [],
  statuses: [
    { key: "todo", name: "Todo", category: "unstarted" },
    { key: "done", name: "Done", category: "completed" },
  ],
  templates: [],
  timezone: "UTC",
  triage: false,
};

function issue(key: string, title: string, extra: Partial<IssueOut> = {}): IssueOut {
  return {
    id: `id-${key}`,
    key,
    title,
    team: "ENG",
    state: "todo",
    category: "unstarted",
    priority: 0,
    aliases: [],
    labels: [],
    links: [],
    relations: [],
    subscribers: [],
    created_at: "2026-10-29T09:00:00Z",
    updated_at: "2026-10-29T09:00:00Z",
    ...extra,
  };
}

const MY_WORK: MyWorkView = {
  open: 2,
  today: "2026-10-29",
  sections: [
    { bucket: "inbox", tasks: [] },
    {
      bucket: "today",
      tasks: [
        task("k1", "Write the launch post", {
          fields: { labels: [], remind: [], due: "2026-10-29" },
        }),
        task("k2", "Fix sign-in", { kind: "issue", issue_key: "ENG-7" }),
        task("D1.b2", "Book flights", {
          fields: { labels: [], remind: [], block: "2026-10-29T15:00/90" },
        }),
      ],
    },
  ],
};

// eslint-disable-next-line unicorn/no-null -- the API's "none"
export const NONE = null;

export const MENTIONED: MyWorkView = {
  open: 1,
  today: "2026-10-29",
  sections: [
    {
      bucket: "inbox",
      tasks: [
        task("mention.c9", "@ana can you check the copy?", {
          kind: "mention",
          issue_key: "ENG-7",
          mentioned_by: "priya",
        }),
      ],
    },
  ],
};

const INBOX: ListView = {
  doc: "list-inbox",
  inbox: true,
  members: [],
  open: 3,
  owner: "ana",
  title: "Inbox",
};
const OPS: ListView = {
  doc: "list-ops",
  inbox: false,
  members: [],
  open: 1,
  owner: "ana",
  title: "Ops",
};
const SHARED: ListView = {
  doc: "list-home",
  inbox: false,
  members: ["ana"],
  open: 4,
  owner: "priya",
  title: "Home",
};

/** A stand-in for the server's parser: "tomorrow 3pm" and "#ops". */
function parseOf(text: string): ParseView {
  const highlights: ParseView["highlights"] = [];
  const due = text.indexOf("tomorrow 3pm");
  if (due !== -1) highlights.push({ start: due, end: due + 12, kind: "due" });
  const list = text.indexOf("#ops");
  if (list !== -1) highlights.push({ start: list, end: list + 4, kind: "list" });
  return {
    title: text,
    assignees: [],
    fields: {
      labels: [],
      remind: [],
      ...(due !== -1 && { due: "2026-10-30T15:00" }),
    },
    highlights,
    ...(list !== -1 && { list: "ops" }),
  };
}

export function fakeApi(overrides: Partial<WorkApi> = {}) {
  const myWork = vi.fn<WorkApi["myWork"]>(() => Promise.resolve(MY_WORK));
  const addTask = vi.fn<WorkApi["addTask"]>((text) => Promise.resolve(task("new", text)));
  const setDone = vi.fn<WorkApi["setDone"]>((id, done) => Promise.resolve(task(id, "x", { done })));
  const createIssue = vi.fn<WorkApi["createIssue"]>((created) =>
    Promise.resolve(issue("ENG-8", created.title)),
  );
  const setState = vi.fn<WorkApi["setState"]>((key, state) =>
    Promise.resolve(issue(key, "Fix sign-in", { state })),
  );
  const addComment = vi.fn<WorkApi["addComment"]>(() => Promise.resolve());
  const dismissMention = vi.fn<WorkApi["dismissMention"]>(() => Promise.resolve());
  const saveSettings = vi.fn<WorkApi["saveSettings"]>((input) =>
    Promise.resolve({
      time_zone: input.time_zone ?? NONE,
      effective_time_zone: input.time_zone ?? "UTC",
      day_starts_at: input.day_starts_at ?? "09:00",
      digest_at: input.digest_at ?? NONE,
    }),
  );
  const move = vi.fn<WorkApi["move"]>((id) => Promise.resolve(task(id, "x")));
  const update = vi.fn<WorkApi["update"]>((id, changes) =>
    Promise.resolve(task(id, changes.title ?? "x")),
  );
  const shareList = vi.fn<WorkApi["shareList"]>((list) =>
    Promise.resolve({ ...OPS, doc: list, members: ["sam"] }),
  );
  const createList = vi.fn<WorkApi["createList"]>((title) =>
    Promise.resolve({ ...OPS, doc: "list-new", title }),
  );
  const syncGoogle = vi.fn<WorkApi["syncGoogle"]>(() =>
    Promise.resolve({ adopted: 0, created: 2, deleted: 0, moved: 1, unblocked: 0, updated: 0 }),
  );
  const api: WorkApi = {
    ...createMemoryCommentsApi("ana"),
    fileIssue: (created) => Promise.resolve({ issue: issue("ENG-9", created.title) }),
    searchIssues: () => Promise.resolve([issue("ENG-7", "Fix sign-in")]),
    setPriority: (key, priority) => Promise.resolve(issue(key, "Fix sign-in", { priority })),
    queryIssues: () => Promise.resolve([{ key: NONE, issues: [issue("ENG-7", "Fix sign-in")] }]),
    inbox: () => Promise.resolve([]),
    triage: (key) => Promise.resolve(issue(key, "x")),
    cycles: () => Promise.resolve({ cycles: [], velocity: NONE }),
    myIssues: () => Promise.resolve({ issues: [] }),
    views: () => Promise.resolve([]),
    createView: () => Promise.reject(new Error("unused")),
    updateView: () => Promise.reject(new Error("unused")),
    deleteView: () => Promise.resolve(),
    teamSettings: () => Promise.reject(new Error("unused")),
    saveTeamSettings: () => Promise.reject(new Error("unused")),
    createTeam: () => Promise.reject(new Error("unused")),
    myWork,
    addTask,
    setDone,
    dismissMention,
    parse: (text) => Promise.resolve(parseOf(text)),
    move,
    update,
    lists: () => Promise.resolve([INBOX, OPS, SHARED]),
    createList,
    shareList,
    google: () => Promise.resolve({ configured: true, connected: true }),
    connectGoogle: () => Promise.resolve("https://accounts.google.com/o/oauth2"),
    syncGoogle,
    disconnectGoogle: () => Promise.resolve(),
    settings: () =>
      Promise.resolve({
        time_zone: NONE,
        effective_time_zone: "UTC",
        day_starts_at: "09:00",
        digest_at: "07:30",
      }),
    saveSettings,
    teams: () => Promise.resolve([TEAM]),
    issues: () => Promise.resolve([issue("ENG-7", "Fix sign-in")]),
    issue: (key) => Promise.resolve(issue(key, "Fix sign-in")),
    createIssue,
    setState,
    comments: () =>
      Promise.resolve([
        {
          id: "c1",
          author: "priya",
          body: "Seen on Safari.",
          by_model: false,
          created_at: "2026-10-29T10:00:00Z",
          deleted: false,
          reactions: [],
        },
      ]),
    addComment,
    ...overrides,
  };
  return {
    api,
    myWork,
    addTask,
    setDone,
    createIssue,
    setState,
    addComment,
    dismissMention,
    saveSettings,
    move,
    update,
    shareList,
    createList,
    syncGoogle,
  };
}

const collab: CollabConnector = {
  open: () => {
    throw new Error("no editor in these tests");
  },
};

export function renderAt(
  path: string,
  api: WorkApi | undefined,
  routes: ReactNode,
  store: WorkspaceStore = createNorthstarStore(),
) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <WorkspaceProvider store={store} collab={collab}>
        <ServerApisProvider apis={{ work: api }}>
          <MemoryRouter initialEntries={[path]}>
            <Routes>{routes}</Routes>
          </MemoryRouter>
        </ServerApisProvider>
      </WorkspaceProvider>
    </I18nProvider>,
  );
  return user;
}
