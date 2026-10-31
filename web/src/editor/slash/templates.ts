// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { JSONContent } from "@tiptap/core";

import { columns, fence, listBlock, p, tabs } from "../content";

/**
 * Starter content for blocks that are empty-looking without it. Each
 * fence starts from the cheat-sheet example so the syntax is discoverable.
 */
export const TEMPLATES: Readonly<Record<string, () => JSONContent>> = {
  columns2: () => columns([p()], [p()]),
  columns3: () => columns([p()], [p()], [p()]),
  tabs: () => tabs(["Overview", p()], ["Details", p()]),
  stats: () =>
    listBlock(
      "stats",
      "94% · Injections caught · ▲ 6 pts",
      "38 ms · p95 latency · ▼ 12 ms",
    ),
  facts: () => listBlock("facts", "Owner: ", "Status: ", "Target: "),
  progress: () => listBlock("progress", "First milestone · 0%"),
  cards: () => listBlock("cards", "🛰 Title — description"),
  steps: () => listBlock("steps", "First step"),
  chart: () =>
    fence(
      "chart",
      "title: Requests per week\ntype: bar\nlabels: [W40, W41, W42, " +
        "W43]\nvalues: [120, 185, 160, 240]\n",
    ),
  timeline: () => fence("timeline", "Oct 6 | Design approved\nNov 24 | GA\n"),
  roadmap: () =>
    fence(
      "roadmap",
      "range: 2026-11-01 .. 2027-06-30\nDiscovery | 2026-11-01 | " +
        "2026-12-15\nLaunch | 2027-05-15 | milestone\n",
    ),
  risk: () =>
    fence("risk-matrix", "PDF export slips | likelihood 3 | impact 4\n"),
  board: () =>
    fence("board", "## To do\n- First task [P1]\n## Doing\n## Done\n"),
  csv: () => fence("table", "Name, Value\nAlpha, 120\nBeta, 90\n", "sortable"),
  architecture: () =>
    fence(
      "architecture",
      "Clients: Web app\nServices: API\nData: Postgres\nWeb app -> API\nAPI " +
        "-> Postgres\n",
    ),
  mermaid: () =>
    fence(
      "mermaid",
      "flowchart LR\n  A[Start] --> B{Decision}\n  B -->|yes| C[Done]\n",
    ),
  calculator: () =>
    fence(
      "calculator",
      "units: Units = 100 [0..1000 step 10]\nprice: Price = 20 [1..100 step " +
        "1]\nresult: Revenue = units * price ($)\n",
    ),
  collection: () => fence("collection", "id: \nview: All\n"),
};
