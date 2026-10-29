// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Chaos writer: creates one page per iteration and logs "ACK <id>" for
// every write the server acknowledged. verify.js later reads each one back:
// an acknowledged write must survive any failure (RPO 0).

import http from 'k6/http';
import { routes } from './routes.js';

const BASE = __ENV.BASE_URL || 'http://localhost:7700';
const SPACE = __ENV.SPACE || 'northstar';
const headers = {
  'Content-Type': 'application/json',
  ...(__ENV.TOKEN ? { Authorization: `Bearer ${__ENV.TOKEN}` } : {}),
};

export const options = {
  scenarios: {
    writes: {
      executor: 'constant-arrival-rate',
      rate: 20,
      timeUnit: '1s',
      duration: __ENV.DURATION || '12m',
      preAllocatedVUs: 20,
    },
  },
  // Failures during a failover are allowed; lost acknowledged writes are not.
  thresholds: { http_req_failed: ['rate<0.05'] },
};

export default function write() {
  const marker = `chaos-${__VU}-${__ITER}`;
  const res = http.post(
    `${BASE}${routes.createPage(SPACE)}`,
    JSON.stringify({ title: marker, body: `# ${marker}\n` }),
    { headers: { ...headers, 'Idempotency-Key': marker } },
  );
  if (res.status === 201) {
    console.log(`ACK ${res.json('id')}`);
  }
}
