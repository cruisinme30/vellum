// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Reads back every acknowledged chaos write (one id per line in ACKS_FILE)
// and fails if any is missing.

import http from 'k6/http';
import { check } from 'k6';
import { routes } from './routes.js';

const BASE = __ENV.BASE_URL || 'http://localhost:7700';
const ids = open(__ENV.ACKS_FILE || '/acks/acks.txt')
  .split('\n')
  .map((line) => line.trim())
  .filter((line) => line.length > 0);
const headers = __ENV.TOKEN ? { Authorization: `Bearer ${__ENV.TOKEN}` } : {};

export const options = { vus: 1, iterations: 1, thresholds: { checks: ['rate==1'] } };

export default function verify() {
  for (const id of ids) {
    const res = http.get(`${BASE}${routes.page(id)}`, { headers });
    check(res, { 'acknowledged write survived': (r) => r.status === 200 });
  }
  console.log(`checked ${ids.length} acknowledged writes`);
}
