// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The nightly load test on the Team profile (ADR 0022, D96).
//
//   k6 run -e BASE_URL=http://localhost:7700 -e TOKEN=... deploy/perf/k6/suite.js
//
// Scenarios mirror the brief's targets: 2,000 reads/s split between cached
// and uncached pages, writes, and 50 concurrent editing sessions. The
// thresholds are the SLOs; `cargo xtask perf-gate` separately fails the run
// when any p95 grows 10 % or more over deploy/perf/baseline.json.

import http from 'k6/http';
import { check, fail } from 'k6';
import { WebSocket } from 'k6/websockets';
import { Trend } from 'k6/metrics';
import { setTimeout } from 'k6/timers';
import { routes } from './routes.js';
import { summarise } from './summary.js';

const BASE = __ENV.BASE_URL || 'http://localhost:7700';
const WS_BASE = BASE.replace(/^http/, 'ws');
const SPACE = __ENV.SPACE || 'northstar';
const PAGES = Number(__ENV.PAGES || 500);
const SCALE = Number(__ENV.SCALE || 1);
const DURATION = __ENV.DURATION || '5m';

const headers = {
  'Content-Type': 'application/json',
  ...(__ENV.TOKEN ? { Authorization: `Bearer ${__ENV.TOKEN}` } : {}),
};

const collabSync = new Trend('collab_sync', true);

export const options = {
  discardResponseBodies: false,
  scenarios: {
    read_cached: {
      executor: 'constant-arrival-rate',
      exec: 'readCached',
      rate: 1600 * SCALE,
      timeUnit: '1s',
      duration: DURATION,
      preAllocatedVUs: 200,
    },
    read_uncached: {
      executor: 'constant-arrival-rate',
      exec: 'readUncached',
      rate: 400 * SCALE,
      timeUnit: '1s',
      duration: DURATION,
      preAllocatedVUs: 100,
    },
    write: {
      executor: 'constant-arrival-rate',
      exec: 'write',
      rate: 50 * SCALE,
      timeUnit: '1s',
      duration: DURATION,
      preAllocatedVUs: 50,
    },
    collab_sync: {
      executor: 'constant-vus',
      exec: 'collab',
      vus: Math.max(1, Math.round(50 * SCALE)),
      duration: DURATION,
    },
  },
  thresholds: {
    'http_req_duration{scenario:read_cached}': ['p(95)<50', 'p(99)<120'],
    'http_req_duration{scenario:read_uncached}': ['p(95)<150'],
    'http_req_duration{scenario:write}': ['p(95)<150'],
    collab_sync: ['p(95)<200'],
    http_req_failed: ['rate<0.001'],
  },
};

const page = (i) => ({
  title: `Northstar load page ${i}`,
  body: `# Northstar load page ${i}\n\nSeeded by the nightly load test.\n`,
});

// Seeds the pages every scenario reads and edits.
export function setup() {
  const ready = http.get(`${BASE}${routes.ready}`);
  if (ready.status !== 200) {
    fail(`${BASE} is not ready: ${ready.status}`);
  }
  const ids = [];
  for (let i = 0; i < PAGES; i += 1) {
    const res = http.post(`${BASE}${routes.createPage(SPACE)}`, JSON.stringify(page(i)), { headers });
    if (!check(res, { 'seed page created': (r) => r.status === 201 })) {
      fail(`seeding failed: ${res.status} ${res.body}`);
    }
    ids.push(res.json('id'));
  }
  return { ids };
}

const pick = (ids) => ids[Math.floor(Math.random() * ids.length)];

// The hot set: the same few pages, served from the render cache.
export function readCached(data) {
  const res = http.get(`${BASE}${routes.page(data.ids[__ITER % 10])}`, { headers });
  check(res, { 'cached read 200': (r) => r.status === 200 });
}

export function readUncached(data) {
  const res = http.get(`${BASE}${routes.page(pick(data.ids))}`, {
    headers: { ...headers, 'Cache-Control': 'no-cache' },
  });
  check(res, { 'uncached read 200': (r) => r.status === 200 });
}

export function write(data) {
  const id = pick(data.ids);
  const body = JSON.stringify({ body: `# Edited\n\nIteration ${__ITER} by VU ${__VU}.\n` });
  const res = http.put(`${BASE}${routes.page(id)}`, body, {
    headers: { ...headers, 'Idempotency-Key': `k6-${__VU}-${__ITER}` },
  });
  check(res, { 'write 200': (r) => r.status === 200 });
}

// One editing session: opens the page's collab socket and times the Yjs
// sync handshake (sync step 1 to sync step 2) for as long as it lasts.
export function collab(data) {
  const ws = new WebSocket(`${WS_BASE}${routes.collab(SPACE, pick(data.ids))}`, null, { headers });
  ws.binaryType = 'arraybuffer';
  let sent = 0;
  const syncStep1 = new Uint8Array([0, 0, 1, 0]).buffer; // empty state vector
  ws.onopen = () => {
    sent = Date.now();
    ws.send(syncStep1);
  };
  ws.onmessage = (event) => {
    const bytes = new Uint8Array(event.data);
    if (bytes[0] === 0 && bytes[1] === 1 && sent > 0) {
      collabSync.add(Date.now() - sent);
      sent = 0;
      setTimeout(() => {
        sent = Date.now();
        ws.send(syncStep1);
      }, 1000);
    }
  };
  setTimeout(() => ws.close(), 30000);
}

export function handleSummary(data) {
  return summarise(data, __ENV.SUMMARY || 'perf-summary.json');
}
