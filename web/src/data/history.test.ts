// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import type { Http } from "../api/http";

import { createHttpPageHistory, ulidTime } from "./history";

describe("ulidTime", () => {
  it("reads the time from a ULID", () => {
    expect(ulidTime("01ARZ3NDEKTSV4RRFFQ69G5FAV")?.getTime()).toBe(
      1_469_922_850_259,
    );
    expect(ulidTime("01arz3ndektsv4rrffq69g5fav")?.getTime()).toBe(
      1_469_922_850_259,
    );
  });

  it("refuses anything else", () => {
    expect(ulidTime("short")).toBeUndefined();
    expect(ulidTime("0LARZ3NDEKTSV4RRFFQ69G5FAV")).toBeUndefined();
  });
});

function client(answer: unknown) {
  const get = vi.fn(() => Promise.resolve(answer));
  const send = vi.fn(() => Promise.resolve({}));
  const http = { get, send, forgetSession: vi.fn() } as unknown as Http;
  return { history: createHttpPageHistory(http), get, send };
}

describe("page history over HTTP", () => {
  const older = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
  const newer = "01BX5ZZKBKACTAV9WEVGEMMVRZ";

  it("lists a page's versions newest first", async () => {
    const { history, get } = client({
      versions: [
        { id: older, size: 10, author: { type: "external" } },
        { id: newer, size: 12, author: { type: "user", user: "u1" } },
      ],
    });
    const list = await history.list("page/1");
    expect(get).toHaveBeenCalledWith("/api/v1/drive/items/page%2F1/versions");
    expect(list.map((version) => version.id)).toEqual([newer, older]);
    expect(list[0]?.author).toEqual({ type: "user", user: "u1" });
  });

  it("reads a version's text, or nothing when it is not text", async () => {
    const text = client({ id: older, content: "# Plan\n" });
    expect(await text.history.read("p", older)).toBe("# Plan\n");
    expect(text.get).toHaveBeenCalledWith(
      `/api/v1/drive/items/p/versions/${older}`,
    );
    const binary = client({
      id: older,
      content: JSON.parse("null") as unknown,
    });
    expect(await binary.history.read("p", older)).toBeUndefined();
  });

  it("restores a version", async () => {
    const { history, send } = client({});
    await history.restore("p", older);
    expect(send).toHaveBeenCalledWith(
      "POST",
      `/api/v1/drive/items/p/versions/${older}/restore`,
    );
  });
});
