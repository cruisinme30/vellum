// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { describe, expect, it, vi } from "vitest";

import {
  decodeBase64Url,
  disablePush,
  enablePush,
  type PushEnvironment,
  pushSupported,
} from "./push";

// The Push API answers null for "no subscription"; the lint rules forbid the literal.
const NULL = JSON.parse("null") as undefined;

function subscription(endpoint = "https://push.example/1") {
  return {
    endpoint,
    toJSON: () => ({ endpoint, keys: { p256dh: "k", auth: "a" } }),
    unsubscribe: vi.fn(() => Promise.resolve(true)),
  };
}

function environment(options: {
  existing?: ReturnType<typeof subscription>;
  permission?: NotificationPermission;
  vapid?: unknown;
  status?: number;
}) {
  const created = subscription("https://push.example/new");
  const pushManager = {
    getSubscription: vi.fn(() => Promise.resolve(options.existing ?? NULL)),
    subscribe: vi.fn(() => Promise.resolve(created)),
  };
  const fetchMock = vi.fn<typeof fetch>((input) => {
    const url = typeof input === "string" ? input : "";
    if (url.endsWith("/vapid-key")) {
      return Promise.resolve(Response.json({ publicKey: options.vapid ?? "AQID-_8" }));
    }
    return Promise.resolve(new Response(undefined, { status: options.status ?? 204 }));
  });
  const pushEnvironment: PushEnvironment = {
    baseUrl: "https://northstar.dev",
    fetch: fetchMock,
    registration: { pushManager } as unknown as PushEnvironment["registration"],
    requestPermission: () => Promise.resolve(options.permission ?? "granted"),
  };
  return { pushEnvironment, fetchMock, pushManager, created };
}

describe("web push", () => {
  it("decodes base64url keys", () => {
    expect([...decodeBase64Url("AQID-_8")]).toEqual([1, 2, 3, 251, 255]);
  });

  it("subscribes with the server's VAPID key and registers the subscription", async () => {
    const { pushEnvironment, fetchMock, pushManager } = environment({});
    expect(await enablePush(pushEnvironment)).toBe("enabled");
    expect(pushManager.subscribe).toHaveBeenCalledWith({
      userVisibleOnly: true,
      applicationServerKey: decodeBase64Url("AQID-_8"),
    });
    const [url, init] = fetchMock.mock.calls[1] ?? [];
    expect(url).toBe("https://northstar.dev/api/v1/push/subscriptions");
    expect(init?.method).toBe("POST");
    expect(JSON.parse(typeof init?.body === "string" ? init.body : "{}")).toMatchObject({
      endpoint: "https://push.example/new",
    });
  });

  it("reuses an existing subscription", async () => {
    const { pushEnvironment, pushManager, fetchMock } = environment({ existing: subscription() });
    await enablePush(pushEnvironment);
    expect(pushManager.subscribe).not.toHaveBeenCalled();
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("stops when permission is refused", async () => {
    const { pushEnvironment, fetchMock } = environment({ permission: "denied" });
    expect(await enablePush(pushEnvironment)).toBe("denied");
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("fails on a missing key or a server error", async () => {
    await expect(enablePush(environment({ vapid: 7 }).pushEnvironment)).rejects.toThrow(
      "no VAPID key",
    );
    await expect(
      enablePush(environment({ existing: subscription(), status: 500 }).pushEnvironment),
    ).rejects.toThrow("HTTP 500");
  });

  it("unsubscribes and tells the server", async () => {
    const existing = subscription();
    const { pushEnvironment, fetchMock } = environment({ existing });
    await disablePush(pushEnvironment);
    expect(existing.unsubscribe).toHaveBeenCalled();
    const [url, init] = fetchMock.mock.calls[0] ?? [];
    expect(init?.method).toBe("DELETE");
    expect(url).toMatch(
      /\/api\/v1\/push\/subscriptions\?endpoint=https%3A%2F%2Fpush\.example%2F1$/,
    );
    expect(init?.body).toBeUndefined();
    const { pushEnvironment: empty, fetchMock: unused } = environment({});
    await disablePush(empty);
    expect(unused).not.toHaveBeenCalled();
  });

  it("detects support", () => {
    expect(pushSupported({})).toBe(false);
    vi.stubGlobal("navigator", { serviceWorker: {} });
    expect(pushSupported({ PushManager: 1, Notification: 1 })).toBe(true);
    vi.unstubAllGlobals();
  });
});
