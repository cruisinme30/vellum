// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/** A top-level destination of the navigation rail (brief §13). */
export interface Destination {
  readonly id: string;
  readonly path: string;
  readonly icon: string;
}

/** Rail order: New, Home, Search, Favorites, Inbox, Data, Theme, Help, Admin. */
export const DESTINATIONS = {
  home: { id: "home", path: "/", icon: "home" },
  search: { id: "search", path: "/search", icon: "search" },
  favorites: { id: "favorites", path: "/favorites", icon: "star" },
  inbox: { id: "inbox", path: "/inbox", icon: "inbox" },
  data: { id: "data", path: "/data", icon: "database" },
  help: { id: "help", path: "/help", icon: "help" },
  admin: { id: "admin", path: "/admin", icon: "admin_panel_settings" },
} as const satisfies Record<string, Destination>;
