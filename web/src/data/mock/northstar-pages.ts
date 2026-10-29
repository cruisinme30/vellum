// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { JSONContent } from "@tiptap/core";

import {
  b,
  body,
  callout,
  code,
  codeBlock,
  columns,
  date,
  details,
  embed,
  fence,
  h,
  hl,
  hr,
  em,
  image,
  issue,
  link,
  linkCard,
  listBlock,
  math,
  mathBlock,
  mention,
  ol,
  p,
  quote,
  table,
  tabs,
  tasks,
  toc,
  ul,
  wiki,
  withIds,
} from "../../editor/content";

import { PAGE } from "./ids";

/** The policy-simulator PRD: one page that shows every block of the dialect. */
const prd = body(
  listBlock("facts", "Owner: Priya S.", "Status: In review", "Target: 2026-11-24"),
  toc(),
  h(2, "Purpose"),
  p(
    "Policy changes at the gateway ship ",
    b("blind"),
    ". The simulator replays ",
    hl("seven days of traffic"),
    " against a draft policy and reports what would change. Owner ",
    mention("priya"),
    ", engineering lead ",
    mention("dev"),
    ".",
  ),
  callout(
    "info",
    p("The gateway is the single entry point; see ", wiki("Gateway architecture"), "."),
  ),
  quote(p("A policy you cannot simulate is a policy you cannot trust.")),
  h(2, "Goals"),
  columns(
    [p(b("Problem")), p("Policy changes ship blind and roll back by hand.")],
    [p(b("Outcome")), p("Every change ships with a diff report attached.")],
  ),
  listBlock(
    "stats",
    "94% · Injections caught · ▲ 6 pts",
    "38 ms · p95 latency · ▼ 12 ms",
    "7 days · Replay window · ▬",
  ),
  h(2, "Requirements"),
  table(
    ["#", "Requirement", "Status"],
    ["R1", "Replay 7 days in under 10 minutes", "Done"],
    ["R2", "Diff report per rule", "In review"],
    ["R3", "PDF report for customers", "Blocked"],
  ),
  p("Replay work is tracked in ", issue("ENG-142"), "; the PDF report in ", issue("ENG-143"), "."),
  listBlock("progress", "Replay engine · 100%", "Diff report · 70%", "PDF report · 10%"),
  h(2, "Plan"),
  fence(
    "roadmap",
    "range: 2026-11-01 .. 2027-06-30\nDiscovery | 2026-11-01 | 2026-12-15\nBuild | 2026-12-01 | 2027-04-30\nLaunch | 2027-05-15 | milestone\n",
  ),
  fence(
    "timeline",
    "Oct 6 | Design approved | Scope: 7-day replay\nOct 20 | Replay in staging\nNov 24 | GA | Blocked on R3\n",
  ),
  fence(
    "chart",
    "title: Blocked requests per week\ntype: bar\nlabels: [W40, W41, W42, W43]\nvalues: [120, 185, 160, 240]\n",
  ),
  fence(
    "risk-matrix",
    "PDF export slips | likelihood 3 | impact 4\nDrive quota | likelihood 2 | impact 2\n",
  ),
  h(2, "How it works"),
  fence(
    "mermaid",
    "flowchart LR\n  A[SDK client] --> B{Gateway}\n  B -->|allowed| C[Backend]\n  B -->|blocked| D[(Audit log)]\n",
  ),
  fence(
    "architecture",
    "Clients: Web app, Mobile app\nServices: API, Worker\nData: Postgres, Object store\nWeb app -> API\nMobile app -> API\nAPI -> Postgres\nWorker -> Object store\n",
  ),
  image("/demo/gateway.svg", "Gateway request path: client, gateway, policy engine, backend"),
  tabs(
    ["Overview", p("The engine evaluates each recorded request against the draft rules.")],
    ["Limits", p("Rule sets are capped at 2,000 rules per space.")],
  ),
  details("Why the audit log is written first", p("So a blocked request still leaves a trace.")),
  p("The F1 score of the classifier is ", math(String.raw`F_1 = \frac{2PR}{P+R}`), ":"),
  mathBlock(String.raw`F_1 = \frac{2PR}{P+R}`),
  fence(
    "calculator",
    "units: Units = 100 [0..1000 step 10]\nprice: Price = 20 [1..100 step 1]\nresult: Revenue = units * price ($)\n",
  ),
  fence(
    "table",
    "Name, Value, Share\nAlpha, 120, 40\nBeta, 90, 30\nGamma, 90, 30\n",
    "sortable bar=Share",
  ),
  h(2, "Rollout"),
  listBlock(
    "steps",
    "Confirm the dashboard is green",
    "Announce in #eng-deploys",
    "Tag the release",
  ),
  tasks(
    [false, "Review the diff report with ", mention("alice"), " ", date("2026-11-03")],
    [true, "Tag the replay engine release"],
  ),
  listBlock("cards", "🛰 Edge listener — terminates TLS.", "⚖️ Policy engine — evaluates rules."),
  linkCard("https://northstar.dev/d/prd-policy-simulator", "PRD: Policy simulator"),
  embed("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
  callout("warn", p("A rewrite of the rule language is ", b("not"), " approved.")),
  callout("good", p("Ship the simulator before the Q4 review.")),
  callout("crit", p("Never roll back the audit writer separately.")),
  callout("tip", p("Press ", code("/"), " to insert any block.")),
  callout("note", p("Last reviewed ", date("today"), ".")),
  hr(),
  p(em("Related: "), link("Northstar status page", "https://status.northstar.dev")),
);

const engHome = body(
  p("Everything the Northstar engineering team writes down lives here."),
  listBlock("pages"),
  h(2, "This week"),
  tasks(
    [true, "Rotate the gateway TLS certificates"],
    [false, "Review ", issue("ENG-142"), " with ", mention("dev")],
    [false, "Draft the Q1 plan ", date("2026-11-10")],
  ),
  callout("tip", p("New here? Start with ", wiki("Welcome to Northstar"), ".")),
);

const gateway = body(
  p("The gateway terminates TLS, authenticates, applies policy and forwards to the backend."),
  fence(
    "mermaid",
    "sequenceDiagram\n  Browser->>Gateway: open page\n  Gateway->>API: cache miss\n  API-->>Gateway: html\n  Gateway-->>Browser: page\n",
  ),
  h(2, "Components"),
  ul("Edge listener", "Policy engine", "Audit writer"),
  codeBlock("bash", "helm rollback gateway 1 -n prod\nkubectl rollout status deploy/gateway\n"),
);

const simple = (_title: string, ...blocks: JSONContent[]) => body(...blocks);

const raw: Record<string, JSONContent> = {
  [PAGE.prd]: prd,
  [PAGE.engHome]: engHome,
  [PAGE.gateway]: gateway,
  [PAGE.rateLimits]: simple(
    "Rate limiting design",
    p("Token buckets per API key, refilled every second."),
    callout("warn", p("Limits apply before authentication to protect the identity service.")),
  ),
  [PAGE.runbook]: simple(
    "Runbook: deploy the gateway",
    listBlock("steps", "Confirm the dashboard is green", "Run the canary", "Promote to 100 %"),
    codeBlock("bash", "kubectl rollout status deploy/gateway -n prod\n"),
  ),
  [PAGE.onCall]: simple(
    "On-call handbook",
    callout("crit", p("This page is outdated; verify every step with ", mention("sam"), ".")),
    ol("Acknowledge the page", "Open the incident channel", "Write the timeline"),
  ),
  [PAGE.roadmap]: simple(
    "Platform roadmap 2027",
    fence(
      "roadmap",
      "range: 2027-01-01 .. 2027-12-31\nSearch v2 | 2027-01-15 | 2027-04-30\nCollab rooms re-home | 2027-03-01 | 2027-06-30\n1.1 | 2027-07-01 | milestone\n",
    ),
  ),
  [PAGE.interviews]: simple(
    "Customer interviews",
    p("Notes from discovery calls."),
    listBlock("pages"),
  ),
  [PAGE.acme]: simple(
    "Interview: Acme Logistics",
    listBlock("facts", "Date: 2026-10-14", "Interviewer: Alice M.", "Segment: Logistics"),
    quote(p("We find out a policy broke when the customer calls us.")),
  ),
  [PAGE.launch]: simple(
    "Launch checklist",
    tasks([false, "Write the launch post"], [false, "Brief support"], [true, "Freeze the scope"]),
  ),
  [PAGE.welcome]: simple(
    "Welcome to Northstar",
    p("Northstar builds the gateway that keeps APIs safe. This handbook explains how we work."),
    listBlock("pages"),
  ),
  [PAGE.onboarding]: simple(
    "Onboarding checklist",
    tasks(
      [false, "Set up your laptop"],
      [false, "Meet your buddy ", mention("sam")],
      [false, "Ship a small change in week one"],
    ),
  ),
  [PAGE.directory]: simple(
    "Team directory",
    table(
      ["Name", "Team", "Handle"],
      ["Priya Shah", "Product", "@priya"],
      ["Dev Rao", "Engineering", "@dev"],
      ["Alice Moreno", "Research", "@alice"],
      ["Sam Okafor", "Platform", "@sam"],
    ),
  ),
};

/** Demo page bodies, with stable block IDs. */
export const NORTHSTAR_BODIES: Readonly<Record<string, JSONContent>> = Object.fromEntries(
  Object.entries(raw).map(([id, content]) => [id, withIds(id, content)]),
);
