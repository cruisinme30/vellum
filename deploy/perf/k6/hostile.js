// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Hostile load (ADR 0030, D121): attack traffic next to ordinary reads.
//
//   k6 run -e BASE_URL=http://localhost:7700 [-e TOKEN=...] \
//     [-e COLLECTION=<collection id>] deploy/perf/k6/hostile.js
//
// Each attack must be refused quickly with a 4xx (or a closed socket),
// never a 5xx, and the ordinary readers next to it must keep their
// latency. The findings these guard are on #259 (RT-001, RT-012, RT-019,
// RT-026, RT-028).

import http from 'k6/http';
import { check } from 'k6';
import { WebSocket } from 'k6/websockets';
import { Counter } from 'k6/metrics';
import { setTimeout } from 'k6/timers';
import { routes } from './routes.js';

const BASE = __ENV.BASE_URL || 'http://localhost:7700';
const WS_BASE = BASE.replace(/^http/, 'ws');
const DURATION = __ENV.DURATION || '1m';
const COLLECTION = __ENV.COLLECTION || '';
// IDs that parse but name nothing: refusals must not depend on real data.
const SPACE = __ENV.SPACE || '01JBQ8Y4ZK6V3W2N5R7T9X1C00';
const DOC = __ENV.DOC || '01JBQ8Y4ZK6V3W2N5R7T9X1C01';

const auth = __ENV.TOKEN ? { Authorization: `Bearer ${__ENV.TOKEN}` } : {};
const json = { 'Content-Type': 'application/json', ...auth };

// A 5xx is a crash or a stall; every attack must avoid one.
const serverErrors = new Counter('hostile_server_errors');
const socketsKept = new Counter('hostile_sockets_kept');

const attack = (exec, rate, vus) => ({
  executor: 'constant-arrival-rate',
  exec,
  rate,
  timeUnit: '1s',
  duration: DURATION,
  preAllocatedVUs: vus,
});

export const options = {
  discardResponseBodies: true,
  scenarios: {
    // The people the server is for, measured while it is attacked.
    readers: attack('reader', 100, 20),
    oversized_bodies: attack('oversizedBody', 5, 10),
    deep_documents: attack('deepDocument', 5, 10),
    wide_queries: attack('wideQuery', 5, 10),
    anonymous_burst: attack('anonymousBurst', 200, 50),
    socket_garbage: {
      executor: 'constant-vus',
      exec: 'socketGarbage',
      vus: 20,
      duration: DURATION,
    },
  },
  thresholds: {
    'http_req_duration{scenario:readers}': ['p(95)<150'],
    'http_req_duration{scenario:oversized_bodies}': ['p(95)<1000'],
    'http_req_duration{scenario:deep_documents}': ['p(95)<500'],
    'http_req_duration{scenario:wide_queries}': ['p(95)<200'],
    hostile_server_errors: ['count==0'],
    hostile_sockets_kept: ['count==0'],
  },
};

const watch = (res, name) => {
  if (res.status >= 500) {
    serverErrors.add(1, { attack: name });
  }
  return res;
};

export function reader() {
  const res = http.get(`${BASE}${routes.ready}`);
  check(res, { 'ready while attacked': (r) => r.status === 200 });
}

// RT-019: bodies past the limit end at the edge with 413, not in memory.
const big = 'x'.repeat(11 * 1024 * 1024);
export function oversizedBody() {
  const res = watch(
    http.post(`${BASE}${routes.createPage(SPACE)}`, big, { headers: json }),
    'oversized_body',
  );
  check(res, { 'oversized body refused': (r) => r.status === 413 || r.status === 401 });
}

// RT-001: a few KB of nesting once overflowed the stack and killed the
// process. Whatever the answer, it must come back, and quickly.
const deep = JSON.stringify({ title: 'Deep', body: `${'> '.repeat(4000)}deep\n` });
export function deepDocument() {
  const res = watch(
    http.post(`${BASE}${routes.createPage(SPACE)}`, deep, { headers: json }),
    'deep_document',
  );
  check(res, { 'deep document answered': (r) => r.status > 0 && r.status < 500 });
}

// RT-026: thousands of filters in one query held the collections service.
const filters = Array.from({ length: 2000 }, () => 'filter=Name%20contains%20zz').join('&');
export function wideQuery() {
  const id = COLLECTION || DOC;
  const res = watch(http.get(`${BASE}${routes.collectionQuery(id)}?${filters}`, { headers: auth }), 'wide_query');
  check(res, { 'wide query refused': (r) => r.status >= 400 && r.status < 500 });
}

// RT-006: anonymous floods are limited, never answered with errors.
export function anonymousBurst() {
  const res = watch(http.get(`${BASE}${routes.page(DOC)}`), 'anonymous_burst');
  check(res, { 'anonymous refused or limited': (r) => [401, 403, 404, 429].includes(r.status) });
}

// RT-012 and RT-028: junk frames close the socket. An unauthenticated
// upgrade never opens one at all.
export function socketGarbage() {
  const ws = new WebSocket(`${WS_BASE}${routes.collab(SPACE, DOC)}`, null, { headers: auth });
  ws.binaryType = 'arraybuffer';
  let opened = false;
  ws.onopen = () => {
    opened = true;
    // A truncated varint, then a claimed 2 GB length.
    ws.send(new Uint8Array([0, 0, 0xff, 0xff, 0xff, 0xff, 0x07]).buffer);
  };
  ws.onerror = () => {};
  setTimeout(() => {
    if (opened && ws.readyState === 1) {
      socketsKept.add(1);
    }
    ws.close();
  }, 3000);
}
