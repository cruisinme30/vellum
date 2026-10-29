// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * A small Northstar drive for auditing the Drive screens (ADR 0052): the
 * answers `vellum serve` gives for Ada's My Drive and the team's shared
 * drive, with long titles to catch overflow.
 */

export const ADA = {
  id: "01M64U0000000000000000ADA0",
  email: "ada@northstar.dev",
  name: "Ada Lovelace",
  handle: "ada",
  admin: false,
};

const SETTINGS = {
  quota_bytes: 10 * 1024 ** 3,
  trash_days: 30,
  keep_all_days: 7,
  daily_days: 30,
  weekly_weeks: 52,
};
const MINE = {
  id: "S1",
  kind: "user_drive",
  slug: "ada",
  name: "My Drive",
  owner: ADA.id,
  settings: SETTINGS,
};
const TEAM = {
  id: "S2",
  kind: "shared_drive",
  slug: "northstar",
  name: "Northstar Product and Engineering",
  settings: { ...SETTINGS, quota_bytes: 100 * 1024 ** 3 },
};

const item = (id: string, kind: string, title: string, fields: object = {}) => ({
  id,
  space_id: "S1",
  kind,
  title,
  path: `ada/${title}${kind === "page" ? ".md" : ""}`,
  size: kind === "folder" ? 0 : 18_432,
  modified: "2026-10-29T17:42:00Z",
  created_by: ADA.id,
  ...fields,
});

const ITEMS = [
  item("F1", "folder", "Launch"),
  item("F2", "folder", "Customer research interviews and synthesis, autumn 2026"),
  item("P1", "page", "Northstar 1.0 launch plan", { parent_id: undefined }),
  item("P2", "page", "Quarterly planning notes with a deliberately long title to wrap"),
  item("B1", "book", "Northstar handbook"),
  item("C1", "collection", "Launch checklist"),
  item("K1", "shortcut", "Roadmap", { target_id: "P1" }),
];

const TRASH = [
  item("T1", "page", "Old pricing draft", { trashed_at: "2026-10-20T09:00:00Z" }),
  item("T2", "folder", "Archive 2025", { trashed_at: "2026-10-27T16:30:00Z" }),
];

const ACTIVITY = {
  events: [
    {
      at: "2026-10-29T17:42:00Z",
      actor: { type: "user", user: ADA.id },
      item_id: "P1",
      kind: "edited",
    },
    {
      at: "2026-10-28T11:05:00Z",
      actor: { type: "user", user: "01M64U0000000000000000GRC0" },
      item_id: "P1",
      kind: "moved",
      from: "northstar/Drafts/Launch plan.md",
      to: "ada/Northstar 1.0 launch plan.md",
    },
  ],
};

/** The JSON for a request path, or undefined for a route the fixture lacks. */
export function driveAnswer(path: string): unknown {
  const routes: [RegExp, unknown][] = [
    [/^\/api\/v1\/drive\/my-drive$/, { space: MINE }],
    [/^\/api\/v1\/drive\/shared-drives$/, { spaces: [TEAM] }],
    [
      /^\/api\/v1\/drive\/shared-with-me$/,
      { items: [item("X1", "page", "Design review", { space_id: "S9" })] },
    ],
    [/^\/api\/v1\/drive\/starred$/, { items: [ITEMS[2]] }],
    [
      /^\/api\/v1\/drive\/recent$/,
      { items: ITEMS.slice(2, 5).map((entry) => ({ item: entry, opened_at: entry.modified })) },
    ],
    [/^\/api\/v1\/drive\/search$/, { items: ITEMS.slice(2, 4) }],
    [/^\/api\/v1\/drive\/spaces\/[^/]+\/items$/, { items: ITEMS }],
    // Only My Drive holds these; another space's tree would show them twice.
    [/^\/api\/v1\/drive\/spaces\/S1\/tree$/, { items: ITEMS.slice(0, 4), truncated: false }],
    [/^\/api\/v1\/drive\/spaces\/[^/]+\/tree$/, { items: [], truncated: false }],
    [/^\/api\/v1\/drive\/spaces\/[^/]+\/trash$/, { items: TRASH }],
    [
      /^\/api\/v1\/drive\/spaces\/[^/]+\/usage$/,
      { used_bytes: 9.4 * 1024 ** 3, quota_bytes: 10 * 1024 ** 3 },
    ],
    [/^\/api\/v1\/drive\/items\/[^/]+\/activity$/, ACTIVITY],
    [/^\/api\/v1\/drive\/items\/[^/]+\/backlinks$/, { items: [ITEMS[3]] }],
    [/^\/api\/v1\/drive\/items\/F1$/, ITEMS[0]],
    // The organisation's look, which the app loads once someone signs in.
    [/^\/api\/v1\/appearance$/, { seed: "#4f5b92", export_theme: "light", site_theme: "auto" }],
  ];
  return routes.find(([pattern]) => pattern.test(path))?.[1];
}
