// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { render, screen, within } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import type { ReactNode } from "react";
import { MemoryRouter, Route, Routes } from "react-router";
import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";
import { createNorthstarStore } from "../data/mock/northstar";
import { ServerApisProvider } from "../data/server-apis";
import type { WorkspaceStore } from "../data/store";
import { WorkspaceProvider } from "../data/workspace-context";
import type { CollabConnector } from "../editor/collab/provider";
import { createMemoryCommentsApi } from "../editor/comments/memory-comments";
import { I18nProvider } from "../i18n/i18n";

import {
  createHttpWorkApi,
  type IssueOut,
  type MyWorkView,
  type TaskView,
  type TeamOut,
  type WorkApi,
} from "./api";
import { IssuePage } from "./issue-page";
import { IssuesPage } from "./issues-page";

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
  archived: false,
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
      ],
    },
  ],
};

// eslint-disable-next-line unicorn/no-null -- the API's "none"
const NONE = null;

function fakeApi(overrides: Partial<WorkApi> = {}) {
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
  const api: WorkApi = {
    ...createMemoryCommentsApi("ana"),
    fileIssue: (created) => Promise.resolve({ issue: issue("ENG-9", created.title) }),
    myWork,
    addTask,
    setDone,
    dismissMention,
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
    searchIssues: () => Promise.resolve([issue("ENG-7", "Fix sign-in")]),
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
    setTeamArchived: () => Promise.reject(new Error("unused")),
    // My Work's own calls are tested in my-work.test.tsx.
    parse: () => Promise.reject(new Error("unused")),
    move: () => Promise.reject(new Error("unused")),
    update: () => Promise.reject(new Error("unused")),
    lists: () => Promise.resolve([]),
    createList: () => Promise.reject(new Error("unused")),
    shareList: () => Promise.reject(new Error("unused")),
    google: () => Promise.resolve({ configured: false, connected: false }),
    connectGoogle: () => Promise.reject(new Error("unused")),
    syncGoogle: () => Promise.reject(new Error("unused")),
    disconnectGoogle: () => Promise.resolve(),
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
  };
}

const collab: CollabConnector = {
  open: () => {
    throw new Error("no editor in these tests");
  },
};

function renderAt(
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

describe("work API", () => {
  it("calls the task and issue routes", async () => {
    const get = vi.fn((path: string) =>
      Promise.resolve(
        path === "/api/v1/work/issues"
          ? { groups: [{ issues: [issue("ENG-1", "a")] }, { issues: [issue("ENG-2", "b")] }] }
          : {
              threads: [
                {
                  comments: [
                    { id: "2", created_at: "2026-10-29T11:00:00Z", deleted: false },
                    { id: "x", created_at: "2026-10-29T09:00:00Z", deleted: true },
                  ],
                },
                { comments: [{ id: "1", created_at: "2026-10-29T10:00:00Z", deleted: false }] },
              ],
            },
      ),
    );
    const send = vi.fn((_method: string, path: string) =>
      Promise.resolve(path.endsWith("/complete") ? { task: task("k1", "a", { done: true }) } : {}),
    );
    const api = createHttpWorkApi({ get, send, forgetSession: vi.fn() } as unknown as Http);

    const issues = await api.issues("ENG");
    expect(issues.map((each) => each.key)).toEqual(["ENG-1", "ENG-2"]);
    expect(get).toHaveBeenLastCalledWith("/api/v1/work/issues", {
      team: "ENG",
      group_by: "none",
      order_by: "priority",
      show_closed: "none",
    });
    const comments = await api.comments("ENG-1");
    expect(comments.map((each) => each.id)).toEqual(["1", "2"]);
    expect(get).toHaveBeenLastCalledWith("/api/v1/work/comments", { target: "ENG-1" });

    const completed = await api.setDone("k1", true);
    expect(completed.done).toBe(true);
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/tasks/k1/complete", {});
    await api.setDone("k1", false);
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/tasks/k1/reopen", {});
    await api.dismissMention("mention.c9");
    expect(send).toHaveBeenLastCalledWith("POST", "/api/v1/my-work/mentions/c9/dismiss", {});
    await api.saveSettings({ time_zone: "Asia/Tokyo", day_starts_at: "09:00", digest_at: NONE });
    expect(send).toHaveBeenLastCalledWith("PUT", "/api/v1/my-work/settings", {
      time_zone: "Asia/Tokyo",
      day_starts_at: "09:00",
      digest_at: NONE,
    });
    await api.setState("ENG-1", "done");
    expect(send).toHaveBeenLastCalledWith("PATCH", "/api/v1/work/issues/ENG-1", {
      state: "done",
    });
  });
});

describe("issues", () => {
  it("lists a team's issues and files a new one", async () => {
    const { api, createIssue } = fakeApi();
    const user = renderAt(
      "/issues",
      api,
      <>
        <Route path="issues" element={<IssuesPage />} />
        <Route path="issues/:issueKey" element={<p>opened</p>} />
      </>,
    );
    expect(await screen.findByRole("link", { name: /ENG-7/ })).toBeDefined();
    await user.type(
      screen.getByRole("textbox", { name: "New issue for Engineering" }),
      "Add dark mode",
    );
    await user.click(screen.getByRole("button", { name: "Create issue" }));
    expect(createIssue).toHaveBeenCalledWith({ team: "ENG", title: "Add dark mode" });
    expect(await screen.findByText("opened")).toBeDefined();
  });

  it("shows an issue, changes its state and adds a comment", async () => {
    const { api, setState, addComment } = fakeApi();
    const user = renderAt(
      "/issues/ENG-7",
      api,
      <Route path="issues/:issueKey" element={<IssuePage />} />,
    );
    expect(await screen.findByRole("heading", { level: 1, name: "Fix sign-in" })).toBeDefined();
    const comments = await screen.findByRole("list");
    expect(within(comments).getByText("Seen on Safari.")).toBeDefined();

    await user.selectOptions(screen.getByRole("combobox", { name: "State" }), "done");
    expect(setState).toHaveBeenCalledWith("ENG-7", "done");

    await user.type(screen.getByRole("textbox", { name: "Add a comment" }), "Fixed in 1.0.1");
    await user.click(screen.getByRole("button", { name: "Comment" }));
    expect(addComment).toHaveBeenCalledWith("ENG-7", "Fixed in 1.0.1");
  });

  it("starts a doc from an issue and opens it", async () => {
    const { api } = fakeApi();
    const store = createNorthstarStore();
    const createPage = vi.spyOn(store, "createPage");
    const user = renderAt(
      "/issues/ENG-7",
      api,
      <>
        <Route path="issues/:issueKey" element={<IssuePage />} />
        <Route path="p/:pageId" element={<p>opened page</p>} />
      </>,
      store,
    );
    await user.click(await screen.findByRole("button", { name: "Create doc from issue" }));
    expect(await screen.findByText("opened page")).toBeDefined();
    expect(createPage).toHaveBeenCalledWith(
      expect.objectContaining({ title: "ENG-7 Fix sign-in", body: "Issue: ENG-7\n" }),
    );
    const created = store.getSnapshot().pages.find((page) => page.title === "ENG-7 Fix sign-in");
    expect(created && store.seedFor(created.id)?.content?.[0]?.content?.[0]?.text).toBe(
      "Issue: ENG-7",
    );
  });

  it("shows a failure with its message", async () => {
    const { api } = fakeApi({ issue: () => Promise.reject(new Error("not found")) });
    renderAt("/issues/ENG-9", api, <Route path="issues/:issueKey" element={<IssuePage />} />);
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("not found");
    expect(screen.getByRole("button", { name: "Copy diagnostic" })).toBeDefined();
  });
});
