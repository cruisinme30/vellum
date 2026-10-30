// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** Shared fixtures for the Work screen tests. */

import { render, screen } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import type { ReactNode } from "react";
import { MemoryRouter, Routes, useLocation } from "react-router";
import { vi } from "vitest";

import { ServerApisProvider } from "../data/server-apis";
import { I18nProvider } from "../i18n/i18n";
import { KeyboardRoot } from "../keyboard/keyboard-root";
import type { CycleOut, GroupOut, IssueOut, NewTeam, TeamOut, ViewOut, WorkApi } from "../work/api";
import type { TeamFile } from "../work/settings/team-file";

// eslint-disable-next-line unicorn/no-null -- the API's "none"
const NONE = null;

export const TEAM: TeamOut = {
  id: "t1",
  key: "ENG",
  name: "Engineering",
  cycles: true,
  default_status: "todo",
  labels: [],
  members: [],
  statuses: [
    { key: "triage", name: "Triage", category: "triage" },
    { key: "todo", name: "Todo", category: "unstarted" },
    { key: "doing", name: "In Progress", category: "started" },
    { key: "done", name: "Done", category: "completed" },
  ],
  templates: [],
  timezone: "UTC",
  triage: true,
  archived: false,
};

/** ENG's settings file, as the settings route returns it. */
export const TEAM_FILE: TeamFile = {
  id: "t1",
  key: "ENG",
  name: "Engineering",
  timezone: "UTC",
  members: ["dev", "priya"],
  last_number: 9,
  workflow: {
    default: "todo",
    statuses: [
      { key: "triage", name: "Triage", category: "triage" },
      { key: "todo", name: "Todo", category: "unstarted" },
      { key: "next", name: "Next", category: "unstarted" },
      { key: "doing", name: "In Progress", category: "started" },
      { key: "done", name: "Done", category: "completed" },
      { key: "canceled", name: "Canceled", category: "canceled" },
      { key: "duplicate", name: "Duplicate", category: "duplicate" },
    ],
  },
  estimates: { scale: "fibonacci", allow_zero: false, unestimated_points: 1 },
  triage: true,
  sla: {
    rules: [
      {
        name: "Urgent",
        when: { field: "priority", op: "is", values: ["urgent"] },
        within_hours: 24,
      },
    ],
  },
  labels: [{ name: "gateway" }],
};

export function issue(key: string, title: string, extra: Partial<IssueOut> = {}): IssueOut {
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

const BOARD: GroupOut[] = [
  { key: "todo", issues: [issue("ENG-1", "Retry storms", { priority: 1 })] },
  { key: "doing", issues: [issue("ENG-2", "Dark mode", { state: "doing" })] },
  { key: "done", issues: [] },
];

export function view(extra: Partial<ViewOut> = {}): ViewOut {
  return {
    id: "v1",
    name: "Urgent board",
    owner: "dev",
    team: "ENG",
    visibility: "team",
    layout: "board",
    group_by: "priority",
    order_by: "due",
    show_closed: "none",
    filter: '{"field":"priority","op":"is","values":["urgent"]}',
    show_sub_issues: true,
    ...extra,
  };
}

export const CYCLE: CycleOut = {
  key: "2026-C22",
  name: NONE,
  starts_on: "2026-10-26",
  ends_on: "2026-11-09",
  completed_points: 5,
  burn_up: [
    { date: "2026-10-26", scope: 10, completed: 0 },
    { date: "2026-10-27", scope: 12, completed: 3 },
    { date: "2026-10-28", scope: 12, completed: 5 },
  ],
};

const unused = () => Promise.reject(new Error("unused"));

export function fakeApi(overrides: Partial<WorkApi> = {}): WorkApi {
  return {
    myWork: unused,
    addTask: unused,
    setDone: unused,
    dismissMention: unused,
    settings: unused,
    saveSettings: unused,
    parse: unused,
    move: unused,
    update: unused,
    lists: unused,
    createList: unused,
    shareList: unused,
    google: unused,
    connectGoogle: unused,
    syncGoogle: unused,
    disconnectGoogle: unused,
    teams: () => Promise.resolve([TEAM]),
    issues: () => Promise.resolve([]),
    searchIssues: unused,
    issue: unused,
    createIssue: unused,
    fileIssue: unused,
    setState: vi.fn((key: string, state: string) => Promise.resolve(issue(key, "x", { state }))),
    setPriority: vi.fn((key: string, priority: number) =>
      Promise.resolve(issue(key, "x", { priority })),
    ),
    comments: unused,
    addComment: unused,
    queryIssues: vi.fn(() => Promise.resolve(BOARD)),
    inbox: vi.fn(() => Promise.resolve([])),
    triage: vi.fn((key: string) => Promise.resolve(issue(key, "x"))),
    cycles: vi.fn(() => Promise.resolve({ cycles: [], velocity: NONE })),
    myIssues: vi.fn(() => Promise.resolve({ issues: [] })),
    views: vi.fn(() => Promise.resolve([])),
    createView: vi.fn(() => Promise.resolve(view())),
    updateView: vi.fn(() => Promise.resolve(view())),
    deleteView: vi.fn(() => Promise.resolve()),
    threads: unused,
    startThread: unused,
    reply: unused,
    editComment: unused,
    deleteComment: unused,
    react: unused,
    resolve: unused,
    assign: unused,
    teamSettings: vi.fn(() => Promise.resolve(TEAM_FILE)),
    saveTeamSettings: vi.fn(() => Promise.resolve(TEAM)),
    createTeam: vi.fn((team: NewTeam) =>
      Promise.resolve({ ...TEAM, id: "t2", key: team.key, name: team.name }),
    ),
    setTeamArchived: vi.fn((key: string, archived: boolean) =>
      Promise.resolve({ ...TEAM, key, archived }),
    ),
    ...overrides,
  };
}

function Where() {
  const location = useLocation();
  return <output data-testid="where">{`${location.pathname}${location.search}`}</output>;
}

export function renderAt(path: string, api: WorkApi, routes: ReactNode) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <ServerApisProvider apis={{ work: api }}>
        <KeyboardRoot>
          <MemoryRouter initialEntries={[path]}>
            <Routes>{routes}</Routes>
            <Where />
          </MemoryRouter>
        </KeyboardRoot>
      </ServerApisProvider>
    </I18nProvider>,
  );
  return user;
}

export const where = () => decodeURIComponent(screen.getByTestId("where").textContent);
