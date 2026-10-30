// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { ApiError, type Http } from "../api/http";

import { createHttpRenderApi } from "./api";

function fakeHttp() {
  const get = vi.fn(() => Promise.resolve({}));
  const send = vi.fn(() => Promise.resolve({ html: "<p>x</p>", warnings: [] }));
  const http = { get, send, forgetSession: vi.fn() } as unknown as Http;
  return { http, get, send };
}

describe("createHttpRenderApi", () => {
  it("reads pages and the stylesheet from the render routes", async () => {
    const { http, get } = fakeHttp();
    const api = createHttpRenderApi(http);
    await api.page("01M6400000PAGE00ROADMAP001");
    await api.stylesheet();
    await api.stylesheet("#C2185B");
    expect(get.mock.calls).toEqual([
      ["/api/v1/pages/01M6400000PAGE00ROADMAP001/render"],
      ["/api/v1/render/stylesheet"],
      ["/api/v1/render/stylesheet?seed=C2185B"],
    ]);
  });

  it("draws each block text once, and asks again after a failure", async () => {
    const { http, send } = fakeHttp();
    const api = createHttpRenderApi(http);
    const block = { lang: "mermaid", args: "", source: "graph TD\n  A --> B\n" };
    await api.block(block);
    await api.block({ ...block });
    expect(send).toHaveBeenCalledTimes(1);
    expect(send).toHaveBeenCalledWith("POST", "/api/v1/render/block", block);

    send.mockRejectedValueOnce(new ApiError("unavailable", "down"));
    const other = { ...block, source: "graph LR\n" };
    await expect(api.block(other)).rejects.toThrow("down");
    await api.block(other);
    expect(send).toHaveBeenCalledTimes(3);
  });
});
