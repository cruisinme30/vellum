// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { newRequestId, newTraceparent, telemetry } from "../telemetry";

/**
 * Web Push for the PWA (R-MW-5). The notifications service owns the VAPID
 * key pair and sends the pushes; this module asks the person, subscribes
 * the browser and registers the subscription:
 *
 * - `GET /api/v1/push/vapid-key` → `{"publicKey": "<base64url>"}`
 * - `POST /api/v1/push/subscriptions` with `PushSubscription.toJSON()`
 * - `DELETE /api/v1/push/subscriptions?endpoint=<url-encoded endpoint>`
 *   (the gateway reads DELETE input from the query string, not the body)
 */

/** Result of {@link enablePush}. */
export type PushState = "enabled" | "denied" | "unsupported";

/** What push needs from the environment; injectable for tests. */
export interface PushEnvironment {
  /** Server origin plus prefix, no trailing slash. */
  readonly baseUrl: string;
  readonly fetch: typeof fetch;
  readonly registration: Pick<ServiceWorkerRegistration, "pushManager">;
  readonly requestPermission: () => Promise<NotificationPermission>;
}

/**
 * Whether this browser can receive pushes. iOS only allows it once the
 * app is added to the Home Screen (iOS 16.4 and later).
 */
export function pushSupported(scope: object = globalThis): boolean {
  return "PushManager" in scope && "Notification" in scope && "serviceWorker" in navigator;
}

/** Decodes the base64url VAPID key into the bytes `subscribe` wants. */
export function decodeBase64Url(value: string): Uint8Array<ArrayBuffer> {
  const padded = value
    .replaceAll("-", "+")
    .replaceAll("_", "/")
    .padEnd(Math.ceil(value.length / 4) * 4, "=");
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1)
    bytes[index] = binary.codePointAt(index) ?? 0;
  return bytes;
}

async function call(
  environment: PushEnvironment,
  path: string,
  init: RequestInit = {},
): Promise<Response> {
  const headers = new Headers(init.headers);
  headers.set("x-request-id", newRequestId());
  headers.set("traceparent", newTraceparent());
  const response = await environment.fetch(`${environment.baseUrl}${path}`, {
    ...init,
    headers,
    credentials: "include",
  });
  if (!response.ok) throw new Error(`Vellum: ${path} answered HTTP ${String(response.status)}`);
  return response;
}

/**
 * Asks for permission (call it from a click), subscribes and registers the
 * subscription with the server. Safe to call again: it reuses an existing
 * subscription.
 */
export async function enablePush(environment: PushEnvironment): Promise<PushState> {
  const permission = await environment.requestPermission();
  if (permission !== "granted") return "denied";
  const { pushManager } = environment.registration;
  let subscription = await pushManager.getSubscription();
  if (subscription === null) {
    const response = await call(environment, "/api/v1/push/vapid-key");
    const { publicKey } = (await response.json()) as { publicKey?: unknown };
    if (typeof publicKey !== "string") throw new Error("Vellum: the server sent no VAPID key");
    subscription = await pushManager.subscribe({
      userVisibleOnly: true,
      applicationServerKey: decodeBase64Url(publicKey),
    });
  }
  await call(environment, "/api/v1/push/subscriptions", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(subscription.toJSON()),
  });
  telemetry().count("vellum.client.push.enabled");
  return "enabled";
}

/** Unsubscribes this browser and tells the server to forget it. */
export async function disablePush(environment: PushEnvironment): Promise<void> {
  const subscription = await environment.registration.pushManager.getSubscription();
  if (subscription === null) return;
  const { endpoint } = subscription;
  await subscription.unsubscribe();
  const query = new URLSearchParams({ endpoint }).toString();
  await call(environment, `/api/v1/push/subscriptions?${query}`, { method: "DELETE" });
}
