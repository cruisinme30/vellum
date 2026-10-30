// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { RoutesView, TraceSummary } from "../../src/admin/requests/api";
import type {
  AdminSpaceList,
  AdminState,
  GroupList,
  UserView,
  AuditList,
  HookList,
  UsageCounts,
  Versioned,
} from "../../src/api/api.gen";

import { EARLIER, NOW, PRIYA } from "./northstar-work";

/**
 * The Northstar admin console's figures, as `vellum serve` answers them (D285).
 */

export const USAGE: UsageCounts = {
  users: 48,
  enabled_users: 45,
  admins: 3,
  active_users_1d: 31,
  active_users_7d: 42,
  active_users_30d: 45,
  api_tokens: 12,
  sessions: 61,
  spaces: 9,
  folders: 140,
  pages: 2318,
  books: 4,
  collections: 11,
  trashed: 27,
  versions: 18_402,
  version_bytes: 1_204_000_000,
  file_bytes: 3_880_000_000,
  events_1d: 1204,
  events_7d: 9877,
  sampled_at_ms: Date.parse(NOW),
};

export const ADMIN_STATE: AdminState = {
  last_telemetry_ms: Date.parse(EARLIER),
};

export const ADMIN_SETTINGS: Versioned = {
  version: 3,
  settings: { organisation: "Northstar" },
};

export const AUDIT: AuditList = {
  records: [
    {
      seq: 2,
      at: NOW,
      hash: "b1946ac92492d2347c6235b4d2611184",
      prev_hash: "6f5902ac237024bdd0c176cb93063dc4",
      event: {
        action: "space.settings.update",
        actor: { kind: "user", user_id: PRIYA, via: "web" },
        details: { trash_days: 30 },
        outcome: "success",
        target: "S2",
      },
    },
    {
      seq: 1,
      at: EARLIER,
      hash: "6f5902ac237024bdd0c176cb93063dc4",
      prev_hash: "0".repeat(32),
      event: {
        action: "auth.sign_in",
        actor: { kind: "user", user_id: PRIYA, via: "password" },
        details: {},
        outcome: "denied",
      },
    },
  ],
};

export const REQUESTS: { requests: TraceSummary[] } = {
  requests: [
    {
      seq: 2,
      trace_id: "4bf92f3577b34da6a3ce929d0e0e4736",
      started_at_ms: Date.parse(NOW),
      duration_us: 18_400,
      name: "GET /api/v1/drive/spaces/{id}/items",
      route: "/api/v1/drive/spaces/{id}/items",
      method: "GET",
      status: 200,
      error: false,
      span_count: 7,
      has_client: true,
    },
    {
      seq: 1,
      trace_id: "00f067aa0ba902b74bf92f3577b34da6",
      started_at_ms: Date.parse(EARLIER),
      duration_us: 912_000,
      name: "POST /api/v1/books/{id}/publish",
      route: "/api/v1/books/{id}/publish",
      method: "POST",
      status: 500,
      error: true,
      span_count: 23,
      has_client: false,
    },
  ],
};

export const ROUTES: RoutesView = {
  minutes: 15,
  in_flight: 2,
  routes: [
    {
      method: "GET",
      route: "/api/v1/drive/spaces/{id}/items",
      rate_per_s: 4.2,
      error_ratio: 0,
      p50_ms: 9,
      p95_ms: 31,
      p99_ms: 58,
      in_flight: 1,
      counts: { requests: 3780, errors: 0 },
      budget_ms: 100,
      over_budget: false,
    },
    {
      method: "POST",
      route: "/api/v1/books/{id}/publish",
      rate_per_s: 0.01,
      error_ratio: 0.25,
      p50_ms: 640,
      p95_ms: 1910,
      p99_ms: 2400,
      in_flight: 1,
      counts: { requests: 8, errors: 2 },
      budget_ms: 1000,
      over_budget: true,
    },
  ],
};

export const HOOKS: HookList = {
  event_types: ["issue.created", "issue.updated", "page.published"],
  hooks: [
    {
      id: "hook1",
      url: "https://hooks.northstar.dev/vellum/issues",
      description: "Issue changes to the on-call bot",
      enabled: true,
      event_types: ["issue.created", "issue.updated"],
      teams: ["ENG"],
      consecutive_failures: 0,
      created_at: EARLIER,
      queued: 0,
      rotating: false,
      secret_hint: "whsec_…4f2a",
    },
  ],
};

export const USERS: UserView[] = [
  {
    id: PRIYA,
    name: "Priya Raman",
    handle: "priya",
    email: "priya@northstar.dev",
    admin: true,
    disabled: false,
    hasPassword: true,
    createdAt: EARLIER,
  },
  {
    id: "01M6409GN2EYWS8Z7NFQQ3SAM0",
    name: "Sam Okafor-Lindqvist",
    handle: "sam",
    email: "sam.okafor-lindqvist@engineering.northstar.dev",
    admin: false,
    disabled: false,
    hasPassword: false,
    createdAt: EARLIER,
  },
  {
    id: "01M6409GN2EYWS8Z7NFQQ3LEE0",
    name: "Lee Chen",
    handle: "lee",
    email: "lee@northstar.dev",
    admin: false,
    disabled: true,
    hasPassword: true,
    createdAt: NOW,
  },
];

export const ADMIN_SPACES: AdminSpaceList = {
  spaces: [
    {
      space: {
        id: "S2",
        kind: "shared_drive",
        slug: "northstar",
        name: "Northstar Product and Engineering",
        settings: {
          daily_days: 30,
          keep_all_days: 7,
          trash_days: 30,
          weekly_weeks: 52,
        },
      },
      used_bytes: 9_400_000_000,
      members: [
        { user_id: PRIYA, role: "owner" },
        { user_id: "01M6409GN2EYWS8Z7NFQQ3SAM0", role: "editor" },
      ],
    },
  ],
};

export const GROUPS: GroupList = {
  groups: [
    {
      id: "01M6400000GROUP0ENGINEER01",
      name: "Engineering",
      description:
        "Everyone who ships code, including contractors on the platform team",
      created_at: EARLIER,
    },
    {
      id: "01M6400000GROUP0DESIGN0001",
      name: "Design",
      description: "",
      created_at: NOW,
    },
  ],
};
