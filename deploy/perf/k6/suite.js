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
// Scenarios mirror the brief's targets (#528, ADR 0022 D299): 2,000 reads/s of
// rendered 50-block pages, split between a hot set and the rest; cold renders
// right after a save; writes; 50 editing sessions in pairs on different
// replicas, timing each keystroke until the other editor sees it; and a steady
// trickle of PDF exports of the same pages (#539, D304), the heaviest request a
// reader can make, which must neither miss the 3 s PDF budget nor slow the
// reads running beside it. The thresholds are the SLOs, and k6 may drop no more
// than 1 % of the reads it was asked to send; `cargo xtask perf-gate`
// separately fails the run when any p95 grows 10 % or more over recent nights.

import http from 'k6/http';
import { check, fail } from 'k6';
import { WebSocket } from 'k6/websockets';
import { Counter, Trend } from 'k6/metrics';
import { clearInterval, setInterval, setTimeout } from 'k6/timers';
import { insertUpdate, stamp, stampsIn, updateFrame } from './keystroke.js';
import { northstarPage } from './pages.js';
import { routes } from './routes.js';
import { base, bases, createPage, savePage, signIn, space } from './session.js';
import { summarise } from './summary.js';

const PAGES = Number(__ENV.PAGES || 500);
const SCALE = Number(__ENV.SCALE || 1);
const DURATION = __ENV.DURATION || '5m';

const collabSync = new Trend('collab_sync', true);
const keystroke = new Trend('keystroke', true);
const renderCold = new Trend('render_cold', true);
const pdfExport = new Trend('pdf_export', true);
// Exports refused because both typesetting slots stayed busy (D231).
const pdfBusy = new Counter('pdf_export_busy');

// Pairs of editors (two sessions each), pages kept for cold renders, and
// the reads k6 may fail to send before the 2,000 reads/s claim is false.
const PAIRS = Math.max(1, Math.round(25 * SCALE));
const COLD_PAGES = 50;
const seconds = (d) => Number(d.slice(0, -1)) * { s: 1, m: 60, h: 3600 }[d.slice(-1)];
const READ_RATE = 2000 * SCALE;
const MAX_DROPPED = Math.max(1, Math.ceil(0.01 * READ_RATE * seconds(DURATION)));

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
    render_cold: {
      executor: 'constant-arrival-rate',
      exec: 'renderAfterSave',
      rate: Math.max(1, Math.round(10 * SCALE)),
      timeUnit: '1s',
      duration: DURATION,
      preAllocatedVUs: 20,
    },
    collab_sync: {
      executor: 'constant-vus',
      exec: 'collab',
      vus: Math.max(1, Math.round(50 * SCALE)),
      duration: DURATION,
    },
    keystroke: {
      executor: 'constant-vus',
      exec: 'keystrokes',
      vus: PAIRS,
      duration: DURATION,
    },
    // Two typesetting slots per replica (D231): one export a second keeps
    // them busy part of the time without queueing for the 30 s slot wait.
    pdf_export: {
      executor: 'constant-arrival-rate',
      exec: 'exportPdf',
      rate: Math.max(1, Math.round(SCALE)),
      timeUnit: '1s',
      duration: DURATION,
      preAllocatedVUs: 10,
    },
  },
  thresholds: {
    'http_req_duration{scenario:read_cached}': ['p(95)<50', 'p(99)<120'],
    'http_req_duration{scenario:read_uncached}': ['p(95)<150'],
    'http_req_duration{scenario:write}': ['p(95)<150'],
    render_cold: ['p(95)<300'],
    collab_sync: ['p(95)<200'],
    keystroke: ['p(95)<200'],
    pdf_export: ['p(95)<3000'],
    pdf_export_busy: ['count<1'],
    'checks{scenario:pdf_export}': ['rate>0.999'],
    'dropped_iterations{scenario:read_cached}': [`count<${MAX_DROPPED}`],
    'dropped_iterations{scenario:read_uncached}': [`count<${MAX_DROPPED}`],
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
    ids.push(createPage(base(i), headers, spaceId, `Northstar handbook ${i}`, northstarPage(i)));
  }
  const cold = [];
  for (let i = 0; i < COLD_PAGES; i += 1) {
    cold.push(createPage(base(i), headers, spaceId, `Northstar cold ${i}`, northstarPage(i)));
  }
  const rooms = [];
  for (let i = 0; i < PAIRS; i += 1) {
    rooms.push(createPage(base(i), headers, spaceId, `Northstar pairing ${i}`, '# Pairing\n'));
  }
  return { headers, spaceId, ids, cold, rooms };
}

const pick = (ids) => ids[Math.floor(Math.random() * ids.length)];
const server = () => base(__VU + __ITER);

// The hot set: the same few rendered pages, from each replica's render
// cache.
export function readCached(data) {
  const res = http.get(`${server()}${routes.render(data.ids[__ITER % 10])}`, {
    headers: data.headers,
    tags: { name: 'render' },
  });
  check(res, { 'cached read 200': (r) => r.status === 200 });
}

// Any of the seeded pages, with no copy in the browser.
export function readUncached(data) {
  const res = http.get(`${server()}${routes.render(pick(data.ids))}`, {
    headers: { ...data.headers, 'Cache-Control': 'no-cache' },
    tags: { name: 'render' },
  });
  check(res, { 'uncached read 200': (r) => r.status === 200 });
}

// A cold render: save a 50-block page, then time only its first render.
export function renderAfterSave(data) {
  const id = data.cold[__ITER % data.cold.length];
  const target = server();
  const saved = savePage(target, data.headers, id, northstarPage(`${__VU}-${__ITER}`), {
    headers: { 'Idempotency-Key': `k6-cold-${__VU}-${__ITER}` },
  });
  if (saved.status !== 200) {
    return; // another writer saved first; nothing new to render
  }
  const res = http.get(`${target}${routes.render(id)}`, {
    headers: data.headers,
    tags: { name: 'render-cold' },
  });
  if (check(res, { 'cold render 200': (r) => r.status === 200 })) {
    renderCold.add(res.timings.duration);
  }
}

// A 50-block Northstar page as PDF, read as the caller. Only the PDF's
// arrival is timed; a refusal because the slots were busy is counted.
export function exportPdf(data) {
  const res = http.get(`${server()}${routes.exportPage(pick(data.ids), 'pdf')}`, {
    headers: data.headers,
    responseType: 'binary',
    tags: { name: 'export-pdf' },
  });
  if (res.status === 503) {
    pdfBusy.add(1);
    return;
  }
  const pdf = check(res, {
    'pdf export 200': (r) => r.status === 200,
    'pdf export is a PDF': (r) =>
      String(r.headers['Content-Type']).startsWith('application/pdf') &&
      r.body.byteLength >= 4 &&
      String.fromCharCode(...new Uint8Array(r.body, 0, 4)) === '%PDF',
  });
  if (pdf) {
    pdfExport.add(res.timings.duration);
  }
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

// One pair of editors in one room, on two replicas when there are two:
// A types a stamped keystroke every 200 ms and B times its arrival. Each
// session lasts 30 s, like a burst of typing, then the pair reconnects.
export function keystrokes(data) {
  const pair = (__VU - 1) % data.rooms.length;
  const room = routes.collab(data.spaceId, data.rooms[pair]);
  const url = (n) => `${base(n).replace(/^http/, 'ws')}${room}`;
  const tag = `${__VU}x${__ITER}`;
  const reader = new WebSocket(url(1), null, { headers: data.headers });
  reader.binaryType = 'arraybuffer';
  reader.onmessage = (event) => {
    const now = Date.now();
    for (const sent of stampsIn(event.data, tag)) keystroke.add(now - sent);
  };
  const writer = new WebSocket(url(0), null, { headers: data.headers });
  writer.binaryType = 'arraybuffer';
  // A client ID of its own per session, as a browser tab would have.
  const client = (__VU * 100003 + __ITER * 7919 + Math.floor(Math.random() * 1e6)) % 2 ** 31;
  let clock = 0;
  let typing;
  const type = () => {
    const text = stamp(tag, Date.now());
    writer.send(updateFrame(insertUpdate(client, clock, text)));
    clock += text.length;
  };
  writer.onopen = () => {
    // Give the reader a moment to join the room before the first key.
    setTimeout(() => {
      typing = setInterval(type, 200);
    }, 500);
  };
  setTimeout(() => {
    clearInterval(typing);
    writer.close();
    reader.close();
  }, 30000);
}

export function handleSummary(data) {
  return summarise(data, __ENV.SUMMARY || 'perf-summary.json');
}
