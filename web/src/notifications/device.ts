// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import type { Http } from "../api/http";
import {
  disablePush,
  enablePush,
  pushSupported,
  type PushState,
} from "../platform/pwa/push";

/** Push on this browser, as the settings screen sees it. */
export interface PushDevice {
  /**
   * False where the browser cannot receive pushes (and on iOS outside the Home
   * Screen).
   */
  readonly supported: boolean;
  /** Whether this browser has a push subscription. */
  subscribed(): Promise<boolean>;
  /** Asks for permission and subscribes; call it from a click. */
  enable(): Promise<PushState>;
  disable(): Promise<void>;
}

/**
 * The real browser, talking to the server at `baseUrl` (no trailing
 * slash). Writes go through `http` when given, so they carry the
 * session's CSRF token.
 */
export function createBrowserPushDevice(baseUrl = "", http?: Http): PushDevice {
  const supported = pushSupported();
  const environment = async () => ({
    baseUrl,
    fetch: globalThis.fetch.bind(globalThis),
    registration: await navigator.serviceWorker.ready,
    requestPermission: () => Notification.requestPermission(),
    ...(http === undefined
      ? {}
      : {
          send: (method: "POST" | "DELETE", path: string, body?: unknown) =>
            http.send(method, path, body),
        }),
  });
  return {
    supported,
    subscribed: async () => {
      if (!supported) return false;
      // A subscription needs notification permission, so without it there
      // is none to look up. Asking anyway is not free: WebKit builds with
      // no push service block the page in getSubscription().
      if (Notification.permission !== "granted") return false;
      const registration = await navigator.serviceWorker.ready;
      return (await registration.pushManager.getSubscription()) !== null;
    },
    enable: async () =>
      supported ? enablePush(await environment()) : "unsupported",
    disable: async () => {
      if (supported) await disablePush(await environment());
    },
  };
}
