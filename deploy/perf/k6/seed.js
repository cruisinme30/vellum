// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Seeds a Northstar workspace for the release drills and the chaos test:
// PAGES pages in SPACE, each created and then edited once.
//
// k6 run -e BASE_URL=http://localhost:7700 -e PAGES=200 deploy/perf/k6/seed.js

import { check } from 'k6';
import { bases, createPage, savePage, signIn, space } from './session.js';

const PAGES = Number(__ENV.PAGES || 200);

export const options = { vus: 1, iterations: 1, thresholds: { checks: ['rate==1'] } };

export default function seed() {
  const headers = signIn(bases[0]);
  const spaceId = space(bases[0], headers);
  for (let i = 0; i < PAGES; i += 1) {
    const title = `Northstar drill page ${i}`;
    const body = `# ${title}\n\n- [ ] Review the Q3 roadmap\n- [x] Ship the release notes\n`;
    const id = createPage(bases[0], headers, spaceId, title, body);
    const edited = savePage(bases[0], headers, id, `${body}\nEdited once.\n`);
    check(edited, { 'page edited': (r) => r.status === 200 });
  }
}
