// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type {
  IssueOut,
  MyWorkSettingsView,
  MyWorkView,
  TaskView,
  ThreadsOut,
} from "../../src/api/api.gen";

/**
 * The Northstar workspace's people, Work issues and My Work tasks, as
 * `vellum serve` answers them (D285). Long titles catch overflow.
 */

export const NOW = "2026-10-29T16:00:00Z";
/** The time the fake's pages are seen at. */
export const NORTHSTAR_NOW = new Date("2026-10-29T17:30:00Z");
export const EARLIER = "2026-10-27T09:30:00Z";
export const PRIYA = "01M6409GN2EYWS8Z7NFQQ3JNWZ";

export const USER = {
  id: PRIYA,
  email: "priya@northstar.dev",
  name: "Priya Raman",
  handle: "priya",
  admin: true,
};

export const TEAM = {
  id: "01M6400000TEAM0ENGINEER01",
  key: "ENG",
  name: "Engineering",
  cycles: true,
  triage: true,
  default_status: "todo",
  labels: ["bug", "feature", "design"],
  lead: "priya",
  members: ["priya", "sam", "lee"],
  templates: [],
  timezone: "America/Los_Angeles",
  archived: false,
  statuses: [
    { key: "backlog", name: "Backlog", category: "backlog" },
    { key: "todo", name: "Todo", category: "unstarted" },
    { key: "in_progress", name: "In progress", category: "started" },
    { key: "done", name: "Done", category: "completed" },
  ],
};

const issue = (
  number: number,
  title: string,
  state: string,
  category: string,
  priority: number,
  extra: Partial<IssueOut> = {},
): IssueOut => ({
  id: `01M6400000ISSUE0000000${String(number).padStart(4, "0")}`,
  key: `ENG-${String(number)}`,
  team: "ENG",
  title,
  state,
  category,
  priority,
  aliases: [],
  labels: [],
  links: [],
  relations: [],
  subscribers: ["priya"],
  created_at: EARLIER,
  updated_at: NOW,
  creator: "priya",
  ...extra,
});

export const ISSUES: IssueOut[] = [
  issue(12, "Sign-in page loops when the session cookie expires", "in_progress", "started", 1, {
    assignee: "priya",
    labels: ["bug"],
    due: "2026-10-31",
    description:
      "After the cookie expires, the sign-in page redirects to itself.\n\n1. Sign in\n2. Wait an hour\n3. Reload",
  }),
  issue(14, "Board view: drag cards between columns", "todo", "unstarted", 2, {
    assignee: "sam",
    labels: ["feature"],
    estimate: "3",
  }),
  issue(
    15,
    "Export a book as PDF with a table of contents, numbered sections and cross-references",
    "todo",
    "unstarted",
    3,
    { labels: ["feature", "design"] },
  ),
  issue(9, "Dark theme contrast on the drive table", "backlog", "backlog", 4),
];

const task = (
  id: string,
  title: string,
  kind: TaskView["kind"],
  extra: Partial<TaskView> = {},
): TaskView => ({
  id,
  title,
  kind,
  done: false,
  overdue: false,
  assignees: ["priya"],
  subtasks: [],
  fields: { labels: [], remind: [] },
  ...extra,
});

export const MY_WORK: MyWorkView = {
  today: "2026-10-29",
  open: 5,
  sections: [
    {
      bucket: "today",
      tasks: [
        task("t1", "Review the 1.0 launch checklist", "checklist", {
          doc: "P1",
          fields: { labels: ["launch"], remind: [], priority: 1, due: "2026-10-29" },
        }),
        task("t2", "Sign-in page loops when the session cookie expires", "issue", {
          issue_key: "ENG-12",
          overdue: true,
          fields: { labels: [], remind: [], due: "2026-10-28" },
        }),
        task("mention.c1", "Sam mentioned you in Roadmap 2027", "mention", {
          mentioned_by: "sam",
          doc: "P1",
        }),
      ],
    },
    {
      bucket: "upcoming",
      day: "2026-11-02",
      tasks: [task("t3", "Write the Q4 OKR retro", "checklist")],
    },
    {
      bucket: "anytime",
      tasks: [
        task("t4", "Read the security self-assessment", "checklist", {
          subtasks: [task("t4a", "Section 3: sessions", "checklist", { done: true })],
        }),
      ],
    },
  ],
};

export const MY_WORK_SETTINGS: MyWorkSettingsView = {
  day_starts_at: "06:00",
  digest_at: "08:00",
  effective_time_zone: "America/Los_Angeles",
  time_zone: "America/Los_Angeles",
};

export const THREADS: ThreadsOut = {
  threads: [
    {
      id: "th1",
      target: { issue: "ENG-12" },
      comments: [
        {
          id: "c1",
          author: "sam",
          body: "I can reproduce this in Safari only.",
          by_model: false,
          created_at: EARLIER,
          deleted: false,
          reactions: [],
        },
        {
          id: "c2",
          author: "priya",
          body: "Thanks, looking at the cookie's SameSite attribute now.",
          by_model: false,
          created_at: NOW,
          deleted: false,
          reactions: [],
        },
      ],
    },
  ],
};
