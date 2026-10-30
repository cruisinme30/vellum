// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { PropertyInfo, Value, ViewInfo } from "./types";
import { bool, date, EMPTY, isEmpty, list, number, text } from "./value";

/**
 * Computes a formula from a row's values: the mock's stand-in for the formula
 * engine.
 */
type Formula = (values: Readonly<Record<string, Value>>) => Value;

export interface MockCollection {
  readonly id: string;
  readonly title: string;
  readonly path: string;
  readonly properties: readonly PropertyInfo[];
  readonly views: readonly ViewInfo[];
  /** Formula functions by property key. */
  readonly formulas: Readonly<Record<string, Formula>>;
}

export interface MockRow {
  readonly id: string;
  readonly values: Readonly<Record<string, Value>>;
  readonly body: string;
}

/** Everything a mock collections store starts from. */
export interface MockSeed {
  readonly collections: readonly MockCollection[];
  readonly rows: Readonly<Record<string, readonly MockRow[]>>;
}

/** Collection and row ids used by the demo and its tests. */
export const COLLECTION = {
  issues: "01JB3KD0COLL0ISSUES0000001",
  releases: "01JB3KD0COLL0RELEASES00002",
} as const;

const RELEASE = {
  beta: "01JB3KD1REL0000000000000B1",
  ga: "01JB3KD1REL0000000000000G1",
  next: "01JB3KD1REL0000000000000N1",
} as const;

const STATUS = [
  { name: "Backlog", color: "gray" },
  { name: "In progress", color: "blue" },
  { name: "In review", color: "purple" },
  { name: "Done", color: "green" },
];
const LABELS = [
  { name: "bug", color: "red" },
  { name: "feature", color: "blue" },
  { name: "docs", color: "yellow" },
  { name: "performance", color: "orange" },
];

const view = (
  name: string,
  kind: ViewInfo["kind"],
  extra: Partial<ViewInfo> = {},
): ViewInfo => ({
  name,
  kind,
  filters: [],
  sorts: [],
  hidden: [],
  ...extra,
});

const pointsLeft: Formula = (values) => {
  const done =
    values["status"]?.type === "text" && values["status"].value === "Done";
  const estimate = values["est"];
  return number(done || estimate?.type !== "number" ? 0 : estimate.value);
};

const ISSUES: MockCollection = {
  id: COLLECTION.issues,
  title: "Issues",
  path: "engineering/issues.collection",
  properties: [
    { key: "name", name: "Issue", type: "title", computed: false },
    {
      key: "status",
      name: "Status",
      type: "select",
      computed: false,
      options: STATUS,
    },
    {
      key: "labels",
      name: "Labels",
      type: "multi",
      computed: false,
      options: LABELS,
    },
    { key: "owner", name: "Owner", type: "person", computed: false },
    { key: "start", name: "Start", type: "date", computed: false },
    { key: "due", name: "Due", type: "date", computed: false },
    {
      key: "est",
      name: "Estimate",
      type: "number",
      computed: false,
      format: "plain",
    },
    { key: "blocked", name: "Blocked", type: "checkbox", computed: false },
    { key: "link", name: "Link", type: "url", computed: false },
    {
      key: "release",
      name: "Release",
      type: "relation",
      computed: false,
      relation: { to: COLLECTION.releases, back: "issues" },
    },
    {
      key: "left",
      name: "Points left",
      type: "formula",
      computed: true,
      formula: 'if(Status == "Done", 0, Estimate)',
      format: "plain",
    },
  ],
  views: [
    view("All issues", "table", { sorts: ["due"] }),
    view("Board", "board", {
      group: "status",
      hidden: ["link", "start", "release"],
    }),
    view("Gallery", "gallery", {
      hidden: ["link", "start", "release", "left"],
    }),
    view("Calendar", "calendar", { date: "due" }),
    view("Timeline", "timeline", {
      date: "start",
      end: "due",
      sorts: ["start"],
    }),
    view("Open", "list", {
      filters: ["status is not Done"],
      sorts: ["est desc"],
    }),
  ],
  formulas: { left: pointsLeft },
};

const RELEASES: MockCollection = {
  id: COLLECTION.releases,
  title: "Releases",
  path: "engineering/releases.collection",
  properties: [
    { key: "name", name: "Release", type: "title", computed: false },
    { key: "ships", name: "Ships", type: "date", computed: false },
    {
      key: "issues",
      name: "Issues",
      type: "relation",
      computed: false,
      relation: { to: COLLECTION.issues, back: "release" },
    },
    {
      key: "points",
      name: "Points",
      type: "rollup",
      computed: true,
      rollup: { via: "issues", prop: "est", agg: "sum" },
      format: "plain",
    },
    {
      key: "count",
      name: "Issue count",
      type: "rollup",
      computed: true,
      rollup: { via: "issues", prop: "name", agg: "count" },
      format: "plain",
    },
  ],
  views: [view("All releases", "table", { sorts: ["ships"] })],
  formulas: {},
};

type IssueSeed = readonly [
  name: string,
  status: string,
  labels: readonly string[],
  owner: string,
  start: string,
  due: string,
  estimate: number | undefined,
  blocked: boolean,
  release: string | undefined,
];

const ISSUE_SEEDS: readonly IssueSeed[] = [
  [
    "Token bucket per tenant",
    "In progress",
    ["feature"],
    "dev",
    "2026-11-02",
    "2026-11-13",
    5,
    false,
    RELEASE.beta,
  ],
  [
    "Gateway returns 502 on cold start",
    "In review",
    ["bug", "performance"],
    "priya",
    "2026-11-04",
    "2026-11-06",
    3,
    false,
    RELEASE.beta,
  ],
  [
    "Document rate-limit headers",
    "Backlog",
    ["docs"],
    "alice",
    "2026-11-16",
    "2026-11-20",
    2,
    false,
    RELEASE.ga,
  ],
  [
    "Replay protection for webhooks",
    "Backlog",
    ["feature"],
    "sam",
    "2026-11-18",
    "2026-12-04",
    8,
    true,
    RELEASE.ga,
  ],
  [
    "Trace sampling knobs",
    "Done",
    ["performance"],
    "dev",
    "2026-10-26",
    "2026-11-03",
    3,
    false,
    RELEASE.beta,
  ],
  [
    "Runbook: drain a region",
    "In progress",
    ["docs"],
    "priya",
    "2026-11-09",
    "2026-11-18",
    2,
    false,
    undefined,
  ],
  [
    "Per-route timeouts",
    "Done",
    ["feature"],
    "alice",
    "2026-10-28",
    "2026-11-05",
    5,
    false,
    RELEASE.beta,
  ],
  [
    "Flaky TLS handshake test",
    "Backlog",
    ["bug"],
    "",
    "",
    "",
    undefined,
    false,
    RELEASE.next,
  ],
];

const issueId = (index: number) =>
  `01JB3KD2ISSUE00000000000${String(index + 10)}`;

const ISSUE_ROWS: readonly MockRow[] = ISSUE_SEEDS.map(
  (
    [name, status, labels, owner, start, due, estimate, blocked, release],
    index,
  ) => ({
    id: issueId(index),
    values: {
      name: text(name),
      status: text(status),
      labels: list(labels.map((label) => text(label))),
      owner: owner === "" ? EMPTY : text(owner),
      start: start === "" ? EMPTY : date(start),
      due: due === "" ? EMPTY : date(due),
      est: estimate === undefined ? EMPTY : number(estimate),
      blocked: bool(blocked),
      link:
        index === 1
          ? text("https://status.northstar.example/incidents/42")
          : EMPTY,
      release: release === undefined ? EMPTY : list([text(release)]),
    },
    body: `Notes on ${name.toLowerCase()}.`,
  }),
);

const RELEASE_ROWS: readonly MockRow[] = [
  [RELEASE.beta, "Gateway beta", "2026-11-14"],
  [RELEASE.ga, "Gateway 1.0", "2026-12-07"],
  [RELEASE.next, "Gateway 1.1", "2027-01-18"],
].map(([id = "", name = "", ships = ""]) => ({
  id,
  values: {
    name: text(name),
    ships: date(ships),
    issues: list(
      ISSUE_ROWS.filter((row) => {
        const release = row.values["release"];
        return !isEmpty(release) && JSON.stringify(release).includes(id);
      }).map((row) => text(row.id)),
    ),
  },
  body: "",
}));

/** The Northstar demo: an issue tracker and the releases it ships in. */
export function northstarCollections(): MockSeed {
  return {
    collections: [ISSUES, RELEASES],
    rows: {
      [COLLECTION.issues]: ISSUE_ROWS,
      [COLLECTION.releases]: RELEASE_ROWS,
    },
  };
}
