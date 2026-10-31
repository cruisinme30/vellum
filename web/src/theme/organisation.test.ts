// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { createHttpOrganisationAppearance } from "./organisation";

describe("createHttpOrganisationAppearance", () => {
  it("reads GET /api/v1/appearance", async () => {
    const get = vi.fn().mockResolvedValue({
      seed: "#b02020",
      export_theme: "sepia",
      site_theme: "auto",
    });
    const load = createHttpOrganisationAppearance({ get });
    await expect(load()).resolves.toEqual({
      seed: "#b02020",
      exportTheme: "sepia",
      siteTheme: "auto",
    });
    expect(get).toHaveBeenCalledWith("/api/v1/appearance");
  });
});
