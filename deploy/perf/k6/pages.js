// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Realistic Northstar pages for the read scenarios (#528, ADR 0022 D299):
// 50 blocks each, cycling through the block kinds a handbook page uses
// (headings, text with marks and links, lists, tasks, a table, code, a
// quote, callouts, maths, details, stats and facts). No diagrams, so
// render times do not depend on the diagram server.

const BLOCKS = [
  (n) => `## Section ${n}: the launch plan`,
  (n) =>
    `Northstar ships **release ${n}** to the _pilot_ teams first; see [the ` +
    `runbook](https://northstar.dev/runbook/${n}) and \`vellum serve\`.`,
  (n) =>
    `- Draft the notes for ${n}\n- Review with design\n  - Copy\n  - ` +
    `Screens\n- Publish`,
  (n) => `- [x] Freeze scope ${n}\n- [ ] Load test\n- [ ] Sign off`,
  (n) =>
    `| Area | Owner | Status |\n| --- | --- | --- |\n| API ${n} | Priya S. | ` +
    `Ready |\n| Web | Ken O. | In review |\n| Docs | Ana M. | Draft |`,
  (n) =>
    `\`\`\`rust\nfn budget_${n}(p95_ms: u64) -> bool {\n    p95_ms < ` +
    `300\n}\n\`\`\``,
  (n) => `> Measure first, then decide. (review ${n})`,
  (n) => `::: info\n\nRelease ${n} keeps the old routes for one cycle.\n\n:::`,
  (n) => `$$\nF_${n} = \\frac{2 \\cdot P \\cdot R}{P + R}\n$$`,
  (n) =>
    `::: details Why release ${n} waits for the load test\n\nSo a slow route ` +
    `is caught before customers see it.\n\n:::`,
];

const EXTRA = [
  `::: stats\n\n- 94% · Pages cached · ▲ 6 pts\n- 38 ms · p95 read · ▼ 12 ` +
    `ms\n\n:::`,
  `::: facts\n\n- Owner: Priya S.\n- Status: In review\n- Target: ` +
    `2026-11-24\n\n:::`,
];

/** A page of `blocks` blocks (50 by default), numbered `i`. */
export function northstarPage(i, blocks = 50) {
  const parts = [`# Northstar handbook ${i}`];
  for (let n = 0; parts.length < blocks; n += 1) {
    parts.push(BLOCKS[n % BLOCKS.length](n));
    if (n % 20 === 19) parts.push(EXTRA[((n / 20) % EXTRA.length) | 0]);
  }
  return `${parts.slice(0, blocks).join("\n\n")}\n`;
}
