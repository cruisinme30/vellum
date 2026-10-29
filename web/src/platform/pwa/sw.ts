// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The service worker entry, built to `/sw.js`. It only binds events; the
 * logic is in `sw-core.ts`. The build replaces `__VELLUM_SHELL__` with the
 * shell manifest (see `vite-plugin.ts`).
 */

import {
  activateShell,
  type ClientsLike,
  installShell,
  openNotice,
  readPush,
  respond,
  route,
  type ShellManifest,
} from "./sw-core";

declare const __VELLUM_SHELL__: ShellManifest;

interface ExtendableEvent extends Event {
  waitUntil(promise: Promise<unknown>): void;
}
interface FetchEvent extends ExtendableEvent {
  readonly request: Request;
  respondWith(response: Promise<Response>): void;
}
interface PushEvent extends ExtendableEvent {
  readonly data: { text(): string } | null;
}
interface NotificationEvent extends ExtendableEvent {
  readonly notification: Notification & { readonly data: unknown };
}
interface WorkerScope {
  readonly location: Location;
  readonly clients: ClientsLike & { claim(): Promise<void> };
  readonly registration: ServiceWorkerRegistration;
  skipWaiting(): Promise<void>;
  addEventListener(type: "install" | "activate", listener: (event: ExtendableEvent) => void): void;
  addEventListener(type: "fetch", listener: (event: FetchEvent) => void): void;
  addEventListener(type: "push", listener: (event: PushEvent) => void): void;
  addEventListener(type: "notificationclick", listener: (event: NotificationEvent) => void): void;
  addEventListener(type: "message", listener: (event: MessageEvent) => void): void;
}

const scope = globalThis as unknown as WorkerScope;
const manifest = __VELLUM_SHELL__;
const origin = scope.location.origin;

scope.addEventListener("install", (event) => {
  event.waitUntil(installShell(caches, manifest));
});

scope.addEventListener("activate", (event) => {
  event.waitUntil(activateShell(caches, manifest).then(() => scope.clients.claim()));
});

scope.addEventListener("message", (event) => {
  if (event.data === "skip-waiting") void scope.skipWaiting();
});

scope.addEventListener("fetch", (event) => {
  const { request } = event;
  if (route(request, origin, manifest).kind === "network") return;
  const answer = respond(request, {
    caches,
    origin,
    manifest,
    fetch: (url) => fetch(url),
  });
  event.respondWith(answer.then((response) => response ?? fetch(request)));
});

scope.addEventListener("push", (event) => {
  const notice = readPush(event.data?.text(), origin);
  const options: NotificationOptions = {
    body: notice.body,
    icon: "/icons/icon-192.png",
    badge: "/icons/icon-192.png",
    data: notice.url,
  };
  if (notice.tag !== undefined) options.tag = notice.tag;
  event.waitUntil(scope.registration.showNotification(notice.title, options));
});

scope.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const path = typeof event.notification.data === "string" ? event.notification.data : "/inbox";
  event.waitUntil(openNotice(scope.clients, origin, path));
});
