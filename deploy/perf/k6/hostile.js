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
// RT-026, RT-028) and round 4 (ADR 0047): sign-in floods, abusive headers
// and URLs. Slow and idle sockets, which k6 cannot hold open byte by
// byte, are in ../hostile/slow_clients.py.

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
    sign_in_flood: attack('signInFlood', 100, 50),
    token_flood: attack('tokenFlood', 50, 20),
    header_bombs: attack('headerBomb', 10, 10),
    huge_urls: attack('hugeUrl', 10, 10),
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
    // Refusals, locked-out callers and a full password gate answer fast;
    // only the few checks let through cost an Argon2 hash.
    'http_req_duration{scenario:sign_in_flood}': ['p(95)<1000'],
    'http_req_duration{scenario:header_bombs}': ['p(95)<1000'],
    'http_req_duration{scenario:huge_urls}': ['p(95)<1000'],
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
  // The server may also close the socket while the 11 MB is still being sent.
  check(res, { 'oversized body refused': (r) => [0, 401, 413].includes(r.status) });
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

// Round 4: a sign-in flood with a new address every time, and a forged
// X-Forwarded-For per request. The forged header must not reset the
// per-client limit or lockout, and the Argon2 work it can cause is
// bounded, so the flood gets 401 or 429 and the readers stay fast.
export function signInFlood() {
  const n = `${__VU}-${__ITER}`;
  const res = watch(
    http.post(
      `${BASE}/auth/sign-in`,
      JSON.stringify({ email: `flood-${n}@northstar.dev`, password: `wrong-password-${n}` }),
      {
        headers: {
          'Content-Type': 'application/json',
          'X-Forwarded-For': `198.51.100.${__ITER % 250}`,
        },
      },
    ),
    'sign_in_flood',
  );
  check(res, { 'sign-in flood refused': (r) => [400, 401, 403, 429].includes(r.status) });
}

// Round 4: bogus OAuth token requests (no CSRF on this cross-site route).
export function tokenFlood() {
  const res = watch(
    http.post(`${BASE}/oauth/token`, `grant_type=authorization_code&code=x${__ITER}&client_id=nope`, {
      headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    }),
    'token_flood',
  );
  check(res, { 'token flood refused': (r) => r.status >= 400 && r.status < 500 });
}

// Round 4: 200 headers of 8 KB each. hyper refuses past its header limit;
// the request must end in a 4xx or a closed socket, never a 5xx.
const bomb = Object.fromEntries(
  Array.from({ length: 200 }, (_, i) => [`X-Bomb-${i}`, 'b'.repeat(8 * 1024)]),
);
export function headerBomb() {
  const res = watch(http.get(`${BASE}${routes.ready}`, { headers: bomb }), 'header_bomb');
  check(res, { 'header bomb refused': (r) => r.status === 0 || (r.status >= 400 && r.status < 500) });
}

// Round 4: a 1 MB URL.
const longPath = `/api/v1/pages/${'p'.repeat(1024 * 1024)}`;
export function hugeUrl() {
  const res = watch(http.get(`${BASE}${longPath}`), 'huge_url');
  check(res, { 'huge URL refused': (r) => r.status === 0 || (r.status >= 400 && r.status < 500) });
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
