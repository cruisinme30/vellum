// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import type { RenderApi } from "./api";
import { ensureRenderStylesheet } from "./rendered-html";

function fakeApi() {
  const stylesheet = vi.fn((seed?: string) =>
    Promise.resolve({
      css: `.vl-render { --seed: ${seed ?? "none"}; }`,
      scope: "vl-render",
      render_version: 3,
    }),
  );
  const api: RenderApi = { page: vi.fn(), block: vi.fn(), stylesheet };
  return { api, stylesheet };
}

const sheets = () => [...document.head.querySelectorAll("style")].map((s) => s.textContent);

describe("ensureRenderStylesheet", () => {
  it("loads one sheet per accent colour and swaps it when the accent changes (D302)", async () => {
    const { api, stylesheet } = fakeApi();
    await ensureRenderStylesheet(api);
    await ensureRenderStylesheet(api);
    expect(stylesheet).toHaveBeenCalledTimes(1);
    expect(sheets()).toEqual([".vl-render { --seed: none; }"]);

    await ensureRenderStylesheet(api, "#c2185b");
    expect(stylesheet).toHaveBeenLastCalledWith("#c2185b");
    expect(sheets()).toEqual([".vl-render { --seed: #c2185b; }"]);
  });

  it("keeps only the newest accent when answers arrive out of order", async () => {
    const { api } = fakeApi();
    const first = ensureRenderStylesheet(api, "#111111");
    const second = ensureRenderStylesheet(api, "#222222");
    const third = ensureRenderStylesheet(api, "#333333");
    await Promise.all([third, second, first]);
    expect(sheets()).toEqual([".vl-render { --seed: #333333; }"]);
  });
});
