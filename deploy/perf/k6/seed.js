// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Seeds a Northstar workspace for the release drills and the chaos test:
// PAGES pages in SPACE, each created and then edited once.
//
//   k6 run -e BASE_URL=http://localhost:7700 -e PAGES=200 deploy/perf/k6/seed.js

import http from 'k6/http';
import { check, fail } from 'k6';
import { routes } from './routes.js';

const BASE = __ENV.BASE_URL || 'http://localhost:7700';
const SPACE = __ENV.SPACE || 'northstar';
const PAGES = Number(__ENV.PAGES || 200);
const headers = {
  'Content-Type': 'application/json',
  ...(__ENV.TOKEN ? { Authorization: `Bearer ${__ENV.TOKEN}` } : {}),
};

export const options = { vus: 1, iterations: 1, thresholds: { checks: ['rate==1'] } };

export default function seed() {
  for (let i = 0; i < PAGES; i += 1) {
    const title = `Northstar drill page ${i}`;
    const body = `# ${title}\n\n- [ ] Review the Q3 roadmap\n- [x] Ship the release notes\n`;
    const created = http.post(`${BASE}${routes.createPage(SPACE)}`, JSON.stringify({ title, body }), {
      headers,
    });
    if (!check(created, { 'page created': (r) => r.status === 201 })) {
      fail(`create ${i}: ${created.status}`);
    }
    const edited = http.put(
      `${BASE}${routes.page(created.json('id'))}`,
      JSON.stringify({ body: `${body}\nEdited once.\n` }),
      { headers },
    );
    check(edited, { 'page edited': (r) => r.status === 200 });
  }
}
