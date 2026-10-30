// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** Fixtures shared by the planning tests: a fake API and a router. */

import { render } from "@testing-library/react";
import { userEvent } from "@testing-library/user-event";
import type { ReactNode } from "react";
import { MemoryRouter, Route, Routes } from "react-router";
import { vi, type Mock } from "vitest";

import { ServerApisProvider } from "../../data/server-apis";
import { I18nProvider } from "../../i18n/i18n";

import type {
  FormOut,
  InsightsOut,
  IssueOut,
  PlanningApi,
  ProjectOut,
  TeamOut,
} from "./api";

const TEAM: TeamOut = {
  id: "t1",
  key: "ENG",
  name: "Engineering",
  cycles: false,
  default_status: "todo",
  labels: [],
  members: [],
  statuses: [{ key: "todo", name: "Todo", category: "unstarted" }],
  templates: ["Bug"],
  timezone: "UTC",
  triage: true,
  archived: false,
};

function project(
  id: string,
  title: string,
  extra: Partial<ProjectOut> = {},
): ProjectOut {
  return {
    id,
    title,
    status: "in-progress",
    teams: ["ENG"],
    members: [],
    milestones: [],
    issues: 4,
    completed_issues: 1,
    scope: 8,
    completed: 2,
    percent: 25,
    update_due: false,
    update_cadence: "weekly",
    markdown: "",
    ...extra,
  };
}

export function issue(
  key: string,
  title: string,
  extra: Partial<IssueOut> = {},
): IssueOut {
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

export const GATEWAY = project("p1", "Gateway replay", {
  health: "at-risk",
  lead: "mara",
  target_date: "2026-12-01",
  update_due: true,
  milestones: [
    { id: "m1", name: "Beta", issues: 2, completed_issues: 1, percent: 50 },
  ],
});

const FORM: FormOut = {
  slug: "bug",
  title: "Report a bug",
  description: "Tell us what broke.",
  public: false,
  fields: [
    {
      key: "summary",
      label: "Summary",
      kind: "text",
      options: [],
      required: true,
      maps_to: "title",
    },
    {
      key: "area",
      label: "Area",
      kind: "select",
      options: ["Web", "API"],
      required: false,
      maps_to: "label",
    },
    {
      key: "details",
      label: "Details",
      kind: "long_text",
      options: [],
      required: false,
      maps_to: "description",
    },
  ],
};

// eslint-disable-next-line unicorn/no-null -- the API's "none"
export const NONE = null;

const INSIGHTS: InsightsOut = {
  overall: {
    count: 12,
    p50_hours: 30,
    p85_hours: 96,
    p95_hours: 120,
    mean_hours: 40,
  },
  buckets: [
    {
      key: "ENG",
      overall: { count: 9, p50_hours: 20, p85_hours: 72 },
      series: [],
    },
    {
      key: NONE,
      overall: { count: 3, p50_hours: 50, p85_hours: 100 },
      series: [],
    },
  ],
};

/** The API with every operation a mock, so tests can look at calls. */
export type FakeApi = { [K in keyof PlanningApi]: Mock<PlanningApi[K]> };

export function fakeApi(overrides: Partial<FakeApi> = {}): FakeApi {
  const api: FakeApi = {
    teams: vi.fn(() => Promise.resolve([TEAM])),
    projects: vi.fn(() =>
      Promise.resolve([
        GATEWAY,
        project("p2", "Old docs", { status: "completed", percent: 100 }),
      ]),
    ),
    project: vi.fn(() => Promise.resolve(GATEWAY)),
    createProject: vi.fn((p) => Promise.resolve(project("p3", p.title))),
    addMilestone: vi.fn(() => Promise.resolve(GATEWAY)),
    projectIssues: vi.fn(() =>
      Promise.resolve([issue("ENG-1", "Record traffic", { milestone: "m1" })]),
    ),
    setMilestone: vi.fn((key) => Promise.resolve(issue(key, "x"))),
    updates: vi.fn(() =>
      Promise.resolve([
        {
          id: "u1",
          author: "mara",
          created_at: "2026-10-28T09:00:00Z",
          health: "at-risk",
          changes: ["Target date moved from 2026-11-15 to 2026-12-01"],
          body: "Replay is slow.",
        },
      ]),
    ),
    postUpdate: vi.fn((_, health, body) =>
      Promise.resolve({
        id: "u2",
        author: "mara",
        created_at: "2026-10-29T09:00:00Z",
        health,
        changes: [],
        body,
      }),
    ),
    initiatives: vi.fn(() =>
      Promise.resolve([
        {
          id: "i1",
          title: "Q4 launch",
          projects: [GATEWAY],
          health: "at-risk",
          percent: 25,
          markdown: "",
        },
      ]),
    ),
    createInitiative: vi.fn(() => Promise.reject(new Error("unused"))),
    templates: vi.fn(() =>
      Promise.resolve([
        {
          name: "Bug",
          labels: ["bug"],
          priority: 2,
          body: "Steps to reproduce:",
          repeat: "1w",
        },
      ]),
    ),
    recurring: vi.fn(() =>
      Promise.resolve([
        issue("ENG-4", "Rotate the on-call pager", {
          due: "2026-11-02",
          repeat: "1w",
        }),
      ]),
    ),
    setRepeat: vi.fn((key, repeat, due) =>
      Promise.resolve(
        issue(
          key,
          "Rotate the on-call pager",
          due === undefined ? { repeat } : { due, repeat },
        ),
      ),
    ),
    createIssue: vi.fn((draft) => Promise.resolve(issue("ENG-9", draft.title))),
    insights: vi.fn(() => Promise.resolve(INSIGHTS)),
    forms: vi.fn(() => Promise.resolve([FORM])),
    submitForm: vi.fn(() => Promise.resolve(issue("ENG-10", "Crash"))),
    brief: vi.fn(() =>
      Promise.resolve({
        key: "ENG-3",
        title: "Crash",
        body: "",
        team: "ENG",
        teams: [],
        similar: [],
        suggestion: {
          team: "OPS",
          priority: 1,
          labels: ["bug"],
          reason: "Looks like the pager outage.",
          model: "Claude",
          at: "2026-10-29T09:00:00Z",
        },
      }),
    ),
    acceptSuggestion: vi.fn((key) => Promise.resolve(issue(key, "Crash"))),
    dismissSuggestion: vi.fn((key) => Promise.resolve(issue(key, "Crash"))),
    ...overrides,
  };
  return api;
}

export function renderAt(
  path: string,
  api: PlanningApi | undefined,
  routes: ReactNode,
) {
  const user = userEvent.setup();
  render(
    <I18nProvider>
      <ServerApisProvider apis={{ planning: api }}>
        <MemoryRouter initialEntries={[path]}>
          <Routes>
            {routes}
            <Route path="/issues/:key" element={<p>issue page</p>} />
          </Routes>
        </MemoryRouter>
      </ServerApisProvider>
    </I18nProvider>,
  );
  return user;
}
