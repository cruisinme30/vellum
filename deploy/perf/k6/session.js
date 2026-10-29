// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// Signing in and finding the shared drive the load tests write to.
//
// With TOKEN set, that API token is used. Otherwise the first run on an
// empty server sets up the administrator (EMAIL, PASSWORD), signs in and
// makes a token; later runs sign in with the same details.

import http from 'k6/http';
import { fail } from 'k6';
import { routes } from './routes.js';

const json = { 'Content-Type': 'application/json' };

// Every base URL the run spreads requests over: BASE_URLS (comma
// separated, one per replica) or BASE_URL.
export const bases = (__ENV.BASE_URLS || __ENV.BASE_URL || 'http://localhost:7700')
  .split(',')
  .map((b) => b.trim())
  .filter((b) => b.length > 0);

// A base URL, taking turns over the replicas.
export const base = (n) => bases[n % bases.length];

// Bearer headers for an API token on `server`.
export function signIn(server) {
  if (__ENV.TOKEN) {
    return { ...json, Authorization: `Bearer ${__ENV.TOKEN}` };
  }
  const email = __ENV.EMAIL || 'load@northstar.dev';
  const password = __ENV.PASSWORD || 'northstar-load-test';
  const setup = http.post(
    `${server}${routes.setup}`,
    JSON.stringify({ email, name: 'Northstar load', password }),
    { headers: json, responseCallback: http.expectedStatuses(200, 409) },
  );
  if (setup.status !== 200 && setup.status !== 409) {
    fail(`setup: ${setup.status} ${setup.body}`);
  }
  const jar = http.cookieJar();
  const signedIn = http.post(`${server}${routes.signIn}`, JSON.stringify({ email, password }), {
    headers: json,
    jar,
  });
  if (signedIn.status !== 200) {
    fail(`sign-in: ${signedIn.status} ${signedIn.body}`);
  }
  const csrf = http.get(`${server}${routes.csrf}`, { jar }).json('token');
  const made = http.post(
    `${server}${routes.tokens}`,
    JSON.stringify({ name: 'k6', scopes: ['read', 'write:direct', 'work'], expiresInDays: 1 }),
    { headers: { ...json, 'x-csrf-token': csrf }, jar },
  );
  if (made.status !== 200) {
    fail(`token: ${made.status} ${made.body}`);
  }
  return { ...json, Authorization: `Bearer ${made.json('token')}` };
}

// The ID of the shared drive with slug SPACE, made if missing.
export function space(server, headers) {
  const slug = __ENV.SPACE || 'northstar';
  const list = http.get(`${server}${routes.sharedDrives}`, { headers });
  const found = (list.json('spaces') || []).find((s) => s.slug === slug);
  if (found) {
    return found.id;
  }
  const made = http.post(
    `${server}${routes.spaces}`,
    JSON.stringify({ kind: 'shared_drive', slug, name: 'Northstar' }),
    { headers },
  );
  if (made.status !== 200) {
    fail(`space: ${made.status} ${made.body}`);
  }
  return made.json('id');
}

// Makes a page and returns its ID.
export function createPage(server, headers, spaceId, title, body) {
  const res = http.post(
    `${server}${routes.createPage(spaceId)}`,
    JSON.stringify({ space_id: spaceId, title, body }),
    { headers },
  );
  if (res.status !== 200) {
    fail(`create ${title}: ${res.status} ${res.body}`);
  }
  return res.json('id');
}

// Saves `content` over a page, starting from what `server` reads now.
// A 409 means another writer saved first, which a load test expects.
export function savePage(server, headers, id, content, params = {}) {
  const read = http.get(`${server}${routes.content(id)}`, { headers, tags: { name: 'content' } });
  if (read.status !== 200) {
    return read;
  }
  return http.put(
    `${server}${routes.content(id)}`,
    JSON.stringify({
      id,
      base_content: read.json('content') || '',
      base_etag: read.json('etag'),
      content,
    }),
    {
      headers: { ...headers, ...(params.headers || {}) },
      tags: { name: 'save' },
      responseCallback: http.expectedStatuses(200, 409),
    },
  );
}
