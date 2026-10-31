// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type {
  FormOut,
  InitiativeOut,
  InsightsOut,
  ProjectOut,
  SharingView,
  TemplateOut,
  UpdateOut,
} from "../../src/api/api.gen";
import type { Snippet } from "../../src/assist/api";

import { PRIYA } from "./northstar-work";

/**
 * Northstar's planning (projects, initiatives, insights, intake forms),
 * the sharing of a page and the MCP setup snippets, for the screens #543
 * added to the UI suite. Long names on purpose, to catch overflow.
 */

export const LAUNCH_PROJECT = "launch";

const DAY = "T09:00:00Z";

export const LAUNCH: ProjectOut = {
  id: LAUNCH_PROJECT,
  title: "Northstar 1.0 launch",
  status: "started",
  health: "at_risk",
  lead: "priya",
  members: ["priya", "ada", "grace"],
  teams: ["ENG"],
  start_date: "2026-09-01",
  target_date: "2026-11-30",
  markdown:
    "Ship 1.0 to the first ten design partners, with sign-in, Drive and Books.",
  issues: 24,
  completed_issues: 15,
  completed: 15,
  scope: 24,
  percent: 63,
  update_cadence: "weekly",
  update_due: true,
  updated_at: `2026-10-28${DAY}`,
  milestones: [
    {
      id: "m1",
      name: "Private beta",
      target_date: "2026-10-15",
      issues: 10,
      completed_issues: 10,
      percent: 100,
    },
    {
      id: "m2",
      name:
        "Public launch with the documentation site and the install " +
        "channels",
      target_date: "2026-11-30",
      issues: 14,
      completed_issues: 5,
      percent: 36,
    },
  ],
};

const MIGRATION: ProjectOut = {
  ...LAUNCH,
  // A planned project: no health and no start date yet.
  health: null, // eslint-disable-line unicorn/no-null -- the wire type
  start_date: null, // eslint-disable-line unicorn/no-null -- the wire type
  id: "migration",
  title:
    "Move every customer workspace off the legacy storage engine before the " +
    "spring",
  status: "planned",
  lead: "grace",
  members: ["grace"],
  target_date: "2027-03-31",
  markdown: "",
  issues: 6,
  completed_issues: 0,
  completed: 0,
  scope: 6,
  percent: 0,
  update_due: false,
  milestones: [],
};

export const PROJECTS: readonly ProjectOut[] = [LAUNCH, MIGRATION];

export const UPDATES: readonly UpdateOut[] = [
  {
    id: "u1",
    author: "priya",
    health: "at_risk",
    created_at: `2026-10-28${DAY}`,
    body:
      "Sign-in loops on an expired session (ENG-12). Everything else is " +
      "on track.",
    changes: ["Target date moved from Nov 15 to Nov 30"],
  },
  {
    id: "u2",
    author: "priya",
    health: "on_track",
    created_at: `2026-10-21${DAY}`,
    body: "Private beta is out to all ten partners.",
    changes: [],
  },
];

export const INITIATIVES: readonly InitiativeOut[] = [
  {
    id: "i1",
    title: "Launch Northstar to design partners",
    owner: "priya",
    health: "at_risk",
    target_date: "2026-12-15",
    markdown: "Everything the first customers need on day one.",
    percent: 42,
    projects: [...PROJECTS],
  },
];

const stats = (count: number, p50: number) => ({
  count,
  mean_hours: p50 * 1.3,
  p50_hours: p50,
  p85_hours: p50 * 2.1,
  p95_hours: p50 * 3.4,
});

const WEEKS = [
  "2026-09-07",
  "2026-09-14",
  "2026-09-21",
  "2026-09-28",
  "2026-10-05",
  "2026-10-12",
];

export const INSIGHTS: InsightsOut = {
  overall: stats(84, 31),
  buckets: [
    {
      overall: stats(84, 31),
      series: WEEKS.map((start, index) => ({
        start,
        stats: stats(10 + index * 2, 40 - index * 3),
      })),
    },
  ],
};

export const TEMPLATES: readonly TemplateOut[] = [
  {
    name: "Bug report",
    title: "Bug: ",
    body: "## What happened\n\n## What you expected\n\n## Steps to reproduce\n",
    labels: ["bug"],
    priority: 2,
    state: "triage",
  },
];

export const FORMS: readonly FormOut[] = [
  {
    slug: "bug-report",
    title: "Report a bug",
    description:
      "Tell the Engineering team about something that does not work as it " +
      "should.",
    public: false,
    fields: [
      {
        key: "summary",
        label: "Summary",
        kind: "text",
        maps_to: "title",
        options: [],
        required: true,
      },
      {
        key: "area",
        label: "Area",
        kind: "select",
        maps_to: "label",
        options: ["Editor", "Drive", "Books", "Sign-in and accounts"],
        required: false,
      },
      {
        key: "details",
        label: "What happened, and what did you expect?",
        kind: "textarea",
        maps_to: "description",
        options: [],
        required: true,
      },
    ],
  },
];

/** Who has Ada's launch plan (drive-fixture.ts, item P1). */
export const SHARING: SharingView = {
  item: {
    id: "P1",
    space_id: "S1",
    kind: "page",
    title: "Northstar 1.0 launch plan",
    path: "ada/Northstar 1.0 launch plan.md",
    size: 18_432,
    modified: "2026-10-29T17:42:00Z",
  },
  grants: [
    { principal: PRIYA, handle: "priya", role: "owner" },
    {
      principal: "01M64U0000000000000000GRC0",
      handle: "grace",
      role: "editor",
    },
    {
      principal: "01M64U0000000000000000LIN0",
      handle: "linus",
      role: "commenter",
      expires_at: "2026-12-01T00:00:00Z",
    },
  ],
  links: [
    {
      id: "L1",
      created_at: "2026-10-27T15:00:00Z",
      created_by: PRIYA,
      expires_at: "2026-11-27T15:00:00Z",
      has_password: true,
      live: true,
    },
  ],
  links_allowed: true,
};

/** `GET /mcp/setup`: how to connect each MCP client. */
export const MCP_SNIPPETS: readonly Snippet[] = [
  {
    client: "claude-code",
    title: "Claude Code",
    instructions: "Run this in a terminal.",
    language: "shell",
    code:
      "claude mcp add --transport http vellum " +
      "https://vellum.northstar.dev/mcp",
  },
  {
    client: "claude-desktop",
    title: "Claude Desktop",
    instructions: "Add this to claude_desktop_config.json.",
    language: "json",
    code: JSON.stringify(
      {
        mcpServers: {
          vellum: { type: "http", url: "https://vellum.northstar.dev/mcp" },
        },
      },
      undefined,
      2,
    ),
  },
];

const PREVIEW_PAGE = [
  '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 210 297">',
  '<rect width="210" height="297" fill="#fff"/>',
  '<text x="20" y="40" font-size="12" font-family="sans-serif">Northstar 1.0 ' +
    "launch plan</text>",
  '<rect x="20" y="55" width="170" height="3" fill="#ccc"/>',
  '<rect x="20" y="65" width="150" height="3" fill="#ccc"/>',
  "</svg>",
].join("");

/** `GET /api/v1/export/pages/{id}?format=preview`: the PDF's first pages. */
export const EXPORT_PREVIEW = {
  title: "Northstar 1.0 launch plan",
  pages: [PREVIEW_PAGE, PREVIEW_PAGE],
  page_count: 2,
  warnings: [],
};
