// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { createMemoryStore, type WorkspaceSeed } from "../store";
import type { PageMeta, Person, Space } from "../types";

import { PAGE } from "./ids";
import { NORTHSTAR_BODIES } from "./northstar-pages";

export { PAGE } from "./ids";

/** The demo workspace from the brief: Northstar (northstar.dev). Fictional. */
export const NORTHSTAR_PEOPLE: readonly Person[] = [
  { handle: "priya", name: "Priya Shah" },
  { handle: "dev", name: "Dev Rao" },
  { handle: "alice", name: "Alice Moreno" },
  { handle: "sam", name: "Sam Okafor" },
  { handle: "team/engineering", name: "Engineering" },
];

const SPACES: readonly Space[] = [
  {
    id: "01JB3K0000SPACE0ENG0000001",
    slug: "engineering",
    name: "Engineering",
    icon: "🛰",
  },
  {
    id: "01JB3K0000SPACE0PRD0000002",
    slug: "product",
    name: "Product",
    icon: "📐",
  },
  {
    id: "01JB3K0000SPACE0HBK0000003",
    slug: "handbook",
    name: "Handbook",
    icon: "📘",
  },
];

const [ENG, PRODUCT, HANDBOOK] = SPACES.map((s) => s.id) as [
  string,
  string,
  string,
];

type Row = [
  id: string,
  space: string,
  parent: string | undefined,
  title: string,
  icon: string,
  status: PageMeta["status"],
  owner: string,
  starred?: boolean,
];

const ROWS: readonly Row[] = [
  [
    PAGE.engHome,
    ENG,
    undefined,
    "Engineering home",
    "🏠",
    "verified",
    "dev",
    true,
  ],
  [
    PAGE.gateway,
    ENG,
    PAGE.engHome,
    "Gateway architecture",
    "🛰",
    "verified",
    "dev",
  ],
  [
    PAGE.rateLimits,
    ENG,
    PAGE.gateway,
    "Rate limiting design",
    "🚦",
    "in-review",
    "alice",
  ],
  [
    PAGE.runbook,
    ENG,
    PAGE.gateway,
    "Runbook: deploy the gateway",
    "🧭",
    "verified",
    "sam",
  ],
  [PAGE.onCall, ENG, PAGE.engHome, "On-call handbook", "📟", "outdated", "sam"],
  [
    PAGE.roadmap,
    ENG,
    undefined,
    "Platform roadmap 2027",
    "🗓",
    "in-review",
    "priya",
    true,
  ],
  [
    PAGE.prd,
    PRODUCT,
    undefined,
    "PRD: Policy simulator",
    "📐",
    "in-review",
    "priya",
    true,
  ],
  [
    PAGE.interviews,
    PRODUCT,
    undefined,
    "Customer interviews",
    "🎙",
    "draft",
    "alice",
  ],
  [
    PAGE.acme,
    PRODUCT,
    PAGE.interviews,
    "Interview: Acme Logistics",
    "🚚",
    "draft",
    "alice",
  ],
  [PAGE.launch, PRODUCT, undefined, "Launch checklist", "✅", "draft", "priya"],
  [
    PAGE.welcome,
    HANDBOOK,
    undefined,
    "Welcome to Northstar",
    "👋",
    "verified",
    "priya",
  ],
  [
    PAGE.onboarding,
    HANDBOOK,
    PAGE.welcome,
    "Onboarding checklist",
    "✅",
    "verified",
    "sam",
  ],
  [
    PAGE.directory,
    HANDBOOK,
    undefined,
    "Team directory",
    "👥",
    "verified",
    "priya",
  ],
];

const PAGES: readonly PageMeta[] = ROWS.map(
  (
    [id, spaceId, parentId, title, icon, status, owner, starred = false],
    index,
  ) => ({
    id,
    spaceId,
    parentId,
    position: index + 1,
    title,
    icon,
    status,
    owner,
    updatedAt: new Date(
      Date.UTC(2026, 9, 28 - (index % 6), 9 + index, 30),
    ).toISOString(),
    starred,
  }),
);

/** The demo seed. */
export const NORTHSTAR: WorkspaceSeed = {
  name: "Northstar",
  me: { handle: "priya", name: "Priya Shah" },
  people: NORTHSTAR_PEOPLE,
  spaces: SPACES,
  pages: PAGES,
  bodies: NORTHSTAR_BODIES,
};

/** A fresh in-memory store holding the Northstar demo. */
export function createNorthstarStore() {
  return createMemoryStore(NORTHSTAR);
}
