// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The HTTP surface the load tests drive, as `vellum openapi` lists it
// (the API is served under /api). A vellum-cli test (perf_routes_tests)
// checks every /api/v1 path here against that document, so a route that
// moves breaks the build instead of the numbers.

export const routes = {
  ready: '/readyz',
  setup: '/api/v1/setup',
  signIn: '/auth/sign-in',
  csrf: '/api/csrf',
  tokens: '/api/v1/me/tokens',
  sharedDrives: '/api/v1/drive/shared-drives',
  spaces: '/api/v1/drive/spaces',
  createPage: (space) => `/api/v1/drive/spaces/${space}/pages`,
  content: (id) => `/api/v1/drive/items/${id}/content`,
  render: (id) => `/api/v1/pages/${id}/render`,
  collectionQuery: (id) => `/api/v1/collections/${id}/query`,
  search: (q) => `/api/v1/search?q=${encodeURIComponent(q)}`,
  issues: '/api/v1/work/issues',
  // y-protocols sync and awareness; extra tags 100-102 can be ignored.
  collab: (space, id) => `/collab/${space}/${id}`,
};
