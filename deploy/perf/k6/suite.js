// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The nightly load test on the Team profile (ADR 0022, D96).
//
//   k6 run -e BASE_URL=http://localhost:7700 deploy/perf/k6/suite.js
//   k6 run -e BASE_URLS=http://localhost:7700,http://localhost:7701 ...
//
// With BASE_URLS, requests take turns over the replicas. TOKEN, EMAIL and
// PASSWORD are as in session.js.
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
import { base, bases, createPage, savePage, signIn, space } from './session.js';
import { summarise } from './summary.js';

const PAGES = Number(__ENV.PAGES || 500);
const SCALE = Number(__ENV.SCALE || 1);
const DURATION = __ENV.DURATION || '5m';

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

// Signs in, finds the space and seeds the pages every scenario reads and
// edits, spread over the replicas.
export function setup() {
  for (const server of bases) {
    const ready = http.get(`${server}${routes.ready}`);
    if (ready.status !== 200) {
      fail(`${server} is not ready: ${ready.status}`);
    }
  }
  const headers = signIn(bases[0]);
  const spaceId = space(bases[0], headers);
  const ids = [];
  for (let i = 0; i < PAGES; i += 1) {
    const body = `# Northstar load page ${i}\n\nSeeded by the nightly load test.\n`;
    ids.push(createPage(base(i), headers, spaceId, `Northstar load page ${i}`, body));
  }
  return { headers, spaceId, ids };
}

const pick = (ids) => ids[Math.floor(Math.random() * ids.length)];
const server = () => base(__VU + __ITER);

// The hot set: the same few pages, served from the render cache.
export function readCached(data) {
  const res = http.get(`${server()}${routes.content(data.ids[__ITER % 10])}`, {
    headers: data.headers,
    tags: { name: 'content' },
  });
  check(res, { 'cached read 200': (r) => r.status === 200 });
}

export function readUncached(data) {
  const res = http.get(`${server()}${routes.content(pick(data.ids))}`, {
    headers: { ...data.headers, 'Cache-Control': 'no-cache' },
    tags: { name: 'content' },
  });
  check(res, { 'uncached read 200': (r) => r.status === 200 });
}

export function write(data) {
  const content = `# Edited\n\nIteration ${__ITER} by VU ${__VU}.\n`;
  const res = savePage(server(), data.headers, pick(data.ids), content, {
    headers: { 'Idempotency-Key': `k6-${__VU}-${__ITER}` },
  });
  check(res, { 'write saved or lost a race': (r) => r.status === 200 || r.status === 409 });
}

// One editing session: opens the page's collab socket and times the Yjs
// sync handshake (sync step 1 to sync step 2) for as long as it lasts.
export function collab(data) {
  const ws = new WebSocket(
    `${server().replace(/^http/, 'ws')}${routes.collab(data.spaceId, pick(data.ids))}`,
    null,
    { headers: data.headers },
  );
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
