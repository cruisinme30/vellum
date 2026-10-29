// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

// The HTTP surface the load tests drive. These paths follow the public API
// as it lands; when a route moves, change it here and nowhere else.

export const routes = {
  ready: '/readyz',
  createPage: (space) => `/api/v1/spaces/${space}/pages`,
  page: (id) => `/api/v1/pages/${id}`,
  // y-protocols sync and awareness; extra tags 100-102 can be ignored.
  collab: (space, id) => `/collab/${space}/${id}`,
};
