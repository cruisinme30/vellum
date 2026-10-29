// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import { Channel } from "../shared/channels.js";

import { bridgeHandlers, dispatch, parseNotice, type BridgeServices } from "./ipc.js";

const signOut = vi.fn<() => Promise<void>>(() => Promise.resolve());
const openExternal = vi.fn<(url: string) => Promise<void>>(() => Promise.resolve());
const notify = vi.fn<BridgeServices["notify"]>();
const privateEnrol = vi.fn<BridgeServices["privateEnrol"]>(() => Promise.resolve({ lines: [] }));

function services(): BridgeServices {
  return {
    serverUrl: () => "https://vellum.northstar.dev",
    signIn: () => Promise.resolve({ account: "ana" }),
    signOut,
    accessToken: () => Promise.resolve("at"),
    openExternal,
    notify,
    privateDeviceKey: () => Promise.resolve({ publicKey: "ab", fingerprint: "cd" }),
    privateEnrol,
    privateReseal: (spaceId) => Promise.resolve({ lines: [String(spaceId)] }),
  };
}

const page = "app://vellum/p/1";

describe("ipc", () => {
  it("answers calls from app pages", async () => {
    const handlers = bridgeHandlers(services());
    expect(await dispatch(handlers, Channel.serverUrl, page)).toBe("https://vellum.northstar.dev");
    expect(await dispatch(handlers, Channel.signIn, page)).toEqual({ account: "ana" });
    expect(await dispatch(handlers, Channel.accessToken, page)).toBe("at");
    await dispatch(handlers, Channel.signOut, page);
    expect(signOut).toHaveBeenCalled();
    await dispatch(handlers, Channel.openExternal, page, "https://northstar.dev");
    expect(openExternal).toHaveBeenCalledWith("https://northstar.dev");
    await dispatch(handlers, Channel.notify, page, { title: "t", body: "b", url: "/p/2?c=1" });
    expect(notify).toHaveBeenCalledWith({ title: "t", body: "b", url: "/p/2?c=1" });
    expect(await dispatch(handlers, Channel.privateDeviceKey, page)).toEqual({
      publicKey: "ab",
      fingerprint: "cd",
    });
    await dispatch(handlers, Channel.privateEnrol, page, { spaceId: "S", label: "L" });
    expect(privateEnrol).toHaveBeenCalledWith("S", "L");
    await dispatch(handlers, Channel.privateEnrol, page, "junk");
    expect(privateEnrol.mock.lastCall).toEqual([undefined, undefined]);
    expect(await dispatch(handlers, Channel.privateReseal, page, "S")).toEqual({ lines: ["S"] });
  });

  it("refuses other senders, unknown channels and unsafe input", async () => {
    const handlers = bridgeHandlers(services());
    await expect(dispatch(handlers, Channel.accessToken, "https://evil.example/")).rejects.toThrow(
      /app pages/,
    );
    await expect(dispatch(handlers, Channel.openUrl, page)).rejects.toThrow(/Unknown channel/);
    await expect(
      dispatch(handlers, Channel.openExternal, page, "file:///etc/passwd"),
    ).rejects.toThrow(TypeError);
    await expect(dispatch(handlers, Channel.openExternal, page, 5)).rejects.toThrow(TypeError);
  });

  it("keeps notification links inside the app", () => {
    expect(parseNotice({ title: "t", body: "b", url: "https://evil.example/x" }).url).toBe(
      "/inbox",
    );
    expect(parseNotice({ title: "x".repeat(300), body: "b", url: "/a" }).title).toHaveLength(200);
    expect(() => parseNotice("nope")).toThrow(TypeError);
    expect(() => parseNotice({ title: "t", body: 1, url: "/" })).toThrow(TypeError);
  });
});
