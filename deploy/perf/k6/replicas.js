// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Two replicas share one state (#441): a write on one is read on the
// other, and how long that takes is measured.
//
//   k6 run -e BASE_URLS=http://localhost:7700,http://localhost:7701 \
//     deploy/perf/k6/replicas.js
//
// Each iteration creates a page on one replica and reads it on the other,
// then saves it on the second and reads the new text on the first. A read
// that has not caught up within LAG_LIMIT seconds fails the run.

import http from 'k6/http';
import { check, fail, sleep } from 'k6';
import { Trend } from 'k6/metrics';
import { routes } from './routes.js';
import { bases, createPage, savePage, signIn, space } from './session.js';
import { summarise } from './summary.js';

const ROUNDS = Number(__ENV.ROUNDS || 50);
const LAG_LIMIT = Number(__ENV.LAG_LIMIT || 5);

const lag = new Trend('replica_lag', true);

export const options = {
  scenarios: {
    replicas: { executor: 'shared-iterations', vus: 5, iterations: ROUNDS, maxDuration: '5m' },
  },
  thresholds: {
    checks: ['rate==1'],
    replica_lag: ['p(95)<1000', 'max<5000'],
  },
};

export function setup() {
  if (bases.length < 2) {
    fail('set BASE_URLS to at least two replicas');
  }
  const headers = signIn(bases[0]);
  return { headers, spaceId: space(bases[0], headers) };
}

// Reads `id` on `server` until `ok(content)` holds; returns the wait in ms.
function readUntil(server, headers, id, ok) {
  const started = Date.now();
  while (Date.now() - started < LAG_LIMIT * 1000) {
    const res = http.get(`${server}${routes.content(id)}`, {
      headers,
      tags: { name: 'content' },
      responseCallback: http.expectedStatuses(200, 404),
    });
    if (res.status === 200 && ok(res.json('content') || '')) {
      return Date.now() - started;
    }
    sleep(0.05);
  }
  return null;
}

export default function round(data) {
  const [a, b] = [bases[__ITER % 2], bases[(__ITER + 1) % 2]];
  const marker = `round ${__VU}-${__ITER}`;
  const id = createPage(a, data.headers, data.spaceId, `Replica check ${marker}`, `# ${marker}\n`);
  const seen = readUntil(b, data.headers, id, (c) => c.includes(marker));
  check(seen, { 'a page made on one replica is read on the other': (ms) => ms !== null });
  if (seen !== null) {
    lag.add(seen);
  }

  const edited = `# ${marker}\n\nEdited on the other replica.\n`;
  const saved = savePage(b, data.headers, id, edited);
  check(saved, { 'the other replica saves it': (r) => r.status === 200 });
  const back = readUntil(a, data.headers, id, (c) => c.includes('Edited on the other replica.'));
  check(back, { 'the edit is read back on the first replica': (ms) => ms !== null });
  if (back !== null) {
    lag.add(back);
  }
}

export function handleSummary(data) {
  return summarise(data, __ENV.SUMMARY || 'replicas-summary.json');
}
