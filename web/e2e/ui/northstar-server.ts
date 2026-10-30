// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { BrowserContext, Route } from "@playwright/test";

import type {
  BookInfo,
  BookList,
  IssueGroups,
  SectionDoc as SectionDocument,
  TeamsOut,
} from "../../src/api/api.gen";
import type { PreferencesWire } from "../../src/notifications/preferences";

import { driveAnswer } from "./drive-fixture";
import {
  ADMIN_SETTINGS,
  ADMIN_SPACES,
  ADMIN_STATE,
  AUDIT,
  GROUPS,
  HOOKS,
  REQUESTS,
  ROUTES,
  USAGE,
  USERS,
} from "./northstar-admin";
import { ISSUES, MY_WORK, MY_WORK_SETTINGS, TEAM, THREADS, USER } from "./northstar-work";

export { NORTHSTAR_NOW } from "./northstar-work";

/**
 * A signed-in Northstar workspace on a stand-in for `vellum serve` (D285):
 * enough of every read API that the server-only screens (Work, My Work,
 * Drive, Books, admin, settings) open with realistic content, so the UI
 * suite audits the screens people see rather than "needs a Vellum
 * server". Reads only: a screen that writes on load is a bug.
 *
 * An API the fake does not know answers 404, which the audit reports as
 * an `http-error`, so a screen that starts calling a new API shows up here.
 */

/** `GET /api/v1/appearance`: the organisation's seed colour and themes. */
export const APPEARANCE = { seed: "#3b5bdb", export_theme: "default", site_theme: "default" };

export const HANDBOOK = "handbook";

const BOOK: BookInfo = {
  id: HANDBOOK,
  title: "Engineering handbook",
  role: "owner",
  numbering: "decimal",
  max_depth: 3,
  toc: [
    { kind: "part", title: "Getting started" },
    {
      id: "welcome",
      label: "welcome",
      title: "Welcome",
      number: "1",
      depth: 0,
      draft: false,
      path: "welcome.md",
      children: [],
    },
    {
      id: "setup",
      label: "setup",
      title: "Setting up your machine",
      number: "2",
      depth: 0,
      draft: false,
      path: "setup.md",
      children: [
        {
          id: "setup-mac",
          label: "setup-mac",
          title: "macOS",
          number: "2.1",
          depth: 1,
          draft: false,
          path: "setup/mac.md",
          children: [],
        },
        {
          id: "setup-linux",
          label: "setup-linux",
          title: "Linux",
          number: "2.2",
          depth: 1,
          draft: true,
          path: "setup/linux.md",
          children: [],
        },
      ],
    },
    { kind: "part", title: "How we work" },
    {
      id: "reviews",
      label: "reviews",
      title: "Code review",
      number: "3",
      depth: 0,
      draft: false,
      path: "reviews.md",
      children: [],
    },
  ],
};

const SECTION: SectionDocument = {
  id: "setup",
  book_id: HANDBOOK,
  label: "setup",
  number: "2",
  title: "Setting up your machine",
  path: "setup.md",
  source: "setup.md",
  restricted: false,
  markdown: [
    "# Setting up your machine",
    "",
    "Install the tools with `scripts/dev/install-tools.sh`, then follow the page for your system.",
    "",
    "- [macOS](setup/mac.md)",
    "- [Linux](setup/linux.md)",
    "",
    "| Tool | Version |",
    "| --- | --- |",
    "| Rust | 1.91 |",
    "| Node | 22 |",
  ].join("\n"),
};

const PREFERENCES: PreferencesWire = {
  channels: {},
  email_delay_minutes: 10,
  muted: [],
};

/** Every GET the fake answers: the path after `/api/v1`, and its answer. */
const READS: readonly (readonly [RegExp, unknown])[] = [
  [/^\/search$/, { hits: [], has_more: false, hybrid: false }],
  [/^\/appearance$/, APPEARANCE],
  [/^\/collections$/, { collections: [] }],
  [/^\/notifications\/preferences$/, PREFERENCES],
  [/^\/my-work$/, MY_WORK],
  [/^\/my-work\/settings$/, MY_WORK_SETTINGS],
  [/^\/work\/teams$/, { teams: [TEAM] } satisfies TeamsOut],
  [/^\/work\/issues$/, { groups: [{ issues: ISSUES }] } satisfies IssueGroups],
  [/^\/work\/comments$/, THREADS],
  [/^\/work\/views$/, { views: [] }],
  [
    /^\/books$/,
    { books: [{ id: HANDBOOK, path: "handbook", title: BOOK.title }] } satisfies BookList,
  ],
  [/^\/books\/[^/]+\/sections\/[^/]+$/, SECTION],
  [/^\/books\/[^/]+\/history$/, { entries: [] }],
  [/^\/books\/[^/]+\/snapshots$/, { snapshots: [] }],
  [/^\/books\/[^/]+\/publications$/, { publications: [] }],
  [/^\/books\/[^/]+$/, BOOK],
  [/^\/admin\/usage$/, USAGE],
  [/^\/admin\/state$/, ADMIN_STATE],
  [/^\/admin\/settings$/, ADMIN_SETTINGS],
  [
    /^\/admin\/backups$/,
    { backups: [{ name: "vellum-2026-10-29T08-00.tar.zst", size: 812_000_000 }] },
  ],
  [/^\/admin\/audit$/, AUDIT],
  [/^\/admin\/requests$/, REQUESTS],
  [/^\/admin\/routes$/, ROUTES],
  [/^\/admin\/webhooks$/, HOOKS],
  [/^\/admin\/users$/, USERS],
  [/^\/admin\/spaces$/, ADMIN_SPACES],
  [/^\/admin\/groups$/, GROUPS],
  [/^\/admin\/groups\/[^/]+\/members$/, { members: [] }],
  [/^\/spaces\/[^/]+\/e2e\/keys$/, { keys: [] }],
];

/** An issue by key, or the Drive fixture's answer (ADR 0052). */
function lookUp(path: string): unknown {
  if (path.startsWith("/drive/")) return driveAnswer(`/api/v1${path}`);
  const key = /^\/work\/issues\/([^/]+)$/.exec(path)?.[1];
  if (key !== undefined) return ISSUES.find((candidate) => candidate.key === key);
  return undefined;
}

function answer(route: Route): Promise<void> {
  const request = route.request();
  const url = new URL(request.url());
  if (request.method() !== "GET") return route.fulfill({ json: {} });
  const path = url.pathname.replace(/^\/api\/v1/, "");
  const found = lookUp(path) ?? READS.find(([pattern]) => pattern.test(path))?.[1];
  if (found === undefined) {
    return route.fulfill({ status: 404, json: { code: "not_found", message: path } });
  }
  return route.fulfill({ json: found });
}

/** Signs Priya in to a Northstar workspace whose read APIs all answer. */
export async function northstarServer(context: BrowserContext): Promise<void> {
  await context.route("**/api/v1/**", answer);
  await context.route("**/api/v1/auth/status", (route) =>
    route.fulfill({
      json: { needsSetup: false, credential: "session", scopes: ["read", "write"], user: USER },
    }),
  );
  await context.route("**/api/csrf", (route) =>
    route.fulfill({ json: { token: "t", header: "x-csrf-token" } }),
  );
}
