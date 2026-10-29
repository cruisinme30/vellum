// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Http } from "../api/http";

import type { OrganisationAppearance } from "./appearance-context";

/** The server's `admin.appearance` answer. */
interface AppearanceResponse {
  readonly seed: string;
  readonly export_theme: string;
  readonly site_theme: string;
}

/** Loads the organisation's look from `GET /api/v1/appearance`. */
export function createHttpOrganisationAppearance(
  http: Pick<Http, "get">,
): () => Promise<OrganisationAppearance> {
  return async () => {
    const value = await http.get<AppearanceResponse>("/api/v1/appearance");
    return { seed: value.seed, exportTheme: value.export_theme, siteTheme: value.site_theme };
  };
}
