// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { BrowserContext } from "@playwright/test";

/**
 * A signed-in Northstar workspace's Work API, for auditing the Work
 * screens (issue #504): one team with triage and cycles, a board's worth of
 * issues with every SLA colour, a saved view and a running cycle.
 */

const TEAM = {
  id: "01JNORTHSTARTEAM0000000000",
  key: "CORE",
  name: "Northstar Core",
  timezone: "America/Los_Angeles",
  lead: "mara",
  members: ["mara", "dev", "priya"],
  statuses: [
    { key: "triage", name: "Triage", category: "triage" },
    { key: "backlog", name: "Backlog", category: "backlog" },
    { key: "todo", name: "Todo", category: "unstarted" },
    { key: "in-progress", name: "In Progress", category: "started" },
    { key: "in-review", name: "In Review", category: "started" },
    { key: "done", name: "Done", category: "completed" },
    { key: "canceled", name: "Canceled", category: "canceled" },
  ],
  default_status: "todo",
  triage: true,
  cycles: true,
  labels: ["gateway", "editor", "sync"],
  templates: [],
};

/** CORE's settings file, as `GET /v1/work/teams/CORE/settings` returns it. */
const SETTINGS = {
  id: TEAM.id,
  key: TEAM.key,
  name: TEAM.name,
  timezone: TEAM.timezone,
  lead: TEAM.lead,
  members: TEAM.members,
  last_number: 40,
  workflow: {
    default: "todo",
    statuses: [...TEAM.statuses, { key: "duplicate", name: "Duplicate", category: "duplicate" }],
  },
  estimates: { scale: "fibonacci", allow_zero: false, unestimated_points: 1 },
  cycles: { duration_weeks: 2, start_day: "monday", cooldown_weeks: 0, upcoming: 3 },
  triage: true,
  triage_rules: [
    {
      name: "Gateway bugs to Dev",
      when: {
        and: [
          { field: "label", op: "is", values: ["gateway"] },
          { field: "title", op: "contains", values: ["timeout"] },
        ],
      },
      // eslint-disable-next-line unicorn/no-thenable -- the field's name in team.yaml
      then: { assignee: "dev", priority: 2 },
    },
  ],
  sla: {
    rules: [
      {
        name: "Urgent",
        when: { field: "priority", op: "is", values: ["urgent"] },
        within_hours: 24,
      },
      { name: "High", when: { field: "priority", op: "is", values: ["high"] }, within_hours: 168 },
    ],
  },
  git: { branch_pushed: "in-progress", pr_opened: "in-review", pr_merged: "done" },
};

const RISKS = ["gray", "yellow", "orange", "red"] as const;

const TITLES = [
  "Retry storms when the S3 backend times out",
  "Dark mode contrast on the board columns",
  "Offline edits lost after a long sleep on macOS",
  "Gateway rejects uppercase origins in the collab socket",
  "Burn-up chart skips weekends in the Pacific zone",
  "Slash menu clipped at 200 % zoom",
  "Export to Word drops table captions",
  "A very long issue title that keeps going to check that cards wrap their text instead of overflowing the column on narrow phones",
];

function issue(n: number, state: string, category: string) {
  const risk = RISKS[n % RISKS.length];
  return {
    id: `01JNORTHSTARISSUE${String(n).padStart(9, "0")}`,
    key: `CORE-${String(n)}`,
    team: "CORE",
    title: TITLES[n % TITLES.length],
    state,
    category,
    priority: n % 5,
    estimate: n % 3 === 0 ? undefined : String((n % 5) + 1),
    assignee: ["mara", "dev", "priya", undefined][n % 4],
    creator: "priya",
    labels: n % 2 === 0 ? ["gateway"] : ["editor"],
    relations: [],
    subscribers: [],
    aliases: [],
    links: [],
    cycle: "2026-C22",
    sla:
      category === "triage" || n % 2 === 0
        ? { kind: "fix", breaches_at: "2026-10-31T17:00:00Z", risk }
        : undefined,
    created_at: "2026-10-20T16:00:00Z",
    updated_at: "2026-10-29T16:00:00Z",
  };
}

const BOARD = TEAM.statuses
  .filter((status) => status.category !== "triage")
  .map((status, column) => ({
    key: status.key,
    issues:
      status.key === "canceled"
        ? []
        : [0, 1].map((row) => issue(column * 2 + row + 10, status.key, status.category)),
  }));

const INBOX = [1, 2, 3].map((n) => issue(n, "triage", "triage"));

const VIEWS = [
  {
    id: "01JNORTHSTARVIEW0000000000",
    name: "Urgent gateway work",
    owner: "mara",
    team: "CORE",
    visibility: "team",
    layout: "board",
    filter:
      '{"and":[{"field":"priority","op":"is","values":["urgent","high"]},{"field":"label","op":"is","values":["gateway"]}]}',
    group_by: "status",
    order_by: "priority",
    show_closed: "past_week",
    show_sub_issues: true,
  },
];

const day = (offset: number) => {
  const date = new Date(Date.UTC(2026, 9, 26 + offset));
  return date.toISOString().slice(0, 10);
};

const CYCLES = {
  velocity: 18.7,
  cycles: [
    {
      key: "2026-C21",
      name: "Fall 2",
      starts_on: "2026-10-12",
      ends_on: "2026-10-26",
      completed_points: 19,
      burn_up: [],
    },
    {
      key: "2026-C22",
      name: "Fall 3",
      starts_on: "2026-10-26",
      ends_on: "2026-11-09",
      completed_points: 8,
      burn_up: [0, 1, 2, 3, 4].map((n) => ({ date: day(n), scope: 20 + n, completed: n * 2 })),
    },
    {
      key: "2026-C23",
      name: undefined,
      starts_on: "2026-11-09",
      ends_on: "2026-11-23",
      completed_points: 0,
      burn_up: [],
    },
  ],
};

const MINE = {
  issues: [
    { focus: "urgent", issue: issue(11, "in-progress", "started") },
    { focus: "sla", issue: issue(12, "todo", "unstarted") },
    { focus: "cycle", issue: issue(13, "in-review", "started") },
    { focus: "backlog", issue: issue(14, "backlog", "backlog") },
  ],
};

const SPACE = {
  id: "01JNORTHSTARSPACE000000000",
  kind: "my_drive",
  name: "My Drive",
  slug: "my-drive",
  owner: "01JNORTHSTARUSER0000000000",
  settings: { daily_days: 30, keep_all_days: 7, trash_days: 30 },
};

const json = (body: unknown) => ({ json: body });

/** Answers the Work API, and an empty My Drive for the signed-in shell. */
export async function workRoutes(context: BrowserContext): Promise<void> {
  await context.route("**/api/v1/drive/**", (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path.endsWith("/my-drive")) return route.fulfill(json({ space: SPACE }));
    if (path.endsWith("/shared-drives")) return route.fulfill(json({ spaces: [] }));
    if (path.endsWith("/tree")) return route.fulfill(json({ items: [], truncated: false }));
    return route.fulfill(json({ items: [] }));
  });
  await context.route("**/api/v1/work/**", (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname.replace(/^\/api\/v1\/work/, "");
    if (path === "/teams") return route.fulfill(json({ teams: [TEAM] }));
    if (path === "/views") return route.fulfill(json({ views: VIEWS }));
    if (path === "/issues") return route.fulfill(json({ groups: BOARD }));
    if (path.endsWith("/inbox"))
      return route.fulfill(json({ groups: [{ key: undefined, issues: INBOX }] }));
    if (path.endsWith("/cycles")) return route.fulfill(json(CYCLES));
    if (path.endsWith("/settings")) return route.fulfill(json({ settings: SETTINGS, yaml: "" }));
    if (path === "/me/issues") return route.fulfill(json(MINE));
    return route.fulfill({ status: 404, json: { code: "not_found", message: path } });
  });
}
