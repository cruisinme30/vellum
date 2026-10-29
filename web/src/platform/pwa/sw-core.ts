// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

/**
 * The service worker's logic, kept free of worker globals so it can be
 * tested. `sw.ts` binds it to the worker's events.
 *
 * Model (ADR 0023, D97): the app shell is precached and served cache-first,
 * so the PWA starts offline and fast; a new deployment ships a new worker
 * whose cache replaces the old one when the person accepts the update.
 * Data never goes through this cache: the local-first store (D98) is the
 * offline path for records and pages.
 */

/** One file of the app shell. Hashed file names need no revision. */
export interface PrecacheEntry {
  readonly url: string;
  /** Content hash for files whose name does not change, such as `/index.html`. */
  readonly revision?: string;
}

/** What the build injects into the worker. */
export interface ShellManifest {
  /** Changes whenever any entry changes. */
  readonly version: string;
  readonly entries: readonly PrecacheEntry[];
}

const PREFIX = "vellum-shell-";

/** Paths that always go to the network: APIs, sign-in and well-known files. */
const NETWORK_ONLY = ["/api/", "/mcp", "/auth/", "/oauth/", "/.well-known/", "/healthz", "/readyz"];

/** The cache that holds this version of the shell. */
export function shellCacheName(manifest: ShellManifest): string {
  return `${PREFIX}${manifest.version}`;
}

/** The URL a precache entry is stored under (revisioned entries get a query). */
export function entryKey(entry: PrecacheEntry): string {
  return entry.revision === undefined ? entry.url : `${entry.url}?__rev=${entry.revision}`;
}

/** Downloads the whole shell. Fails (and the install with it) if any file fails. */
export async function installShell(caches: CacheStorage, manifest: ShellManifest): Promise<void> {
  const cache = await caches.open(shellCacheName(manifest));
  await cache.addAll(manifest.entries.map((entry) => entryKey(entry)));
}

/** Deletes every older shell cache. */
export async function activateShell(
  caches: CacheStorage,
  manifest: ShellManifest,
): Promise<string[]> {
  const current = shellCacheName(manifest);
  const names = await caches.keys();
  const stale = names.filter((name) => name.startsWith(PREFIX) && name !== current);
  await Promise.all(stale.map((name) => caches.delete(name)));
  return stale;
}

/** The parts of a request routing looks at. */
export interface RouteRequest {
  readonly url: string;
  readonly method: string;
  readonly mode: string;
}

/** How the worker answers a request. */
export type Route =
  | { readonly kind: "network" }
  | { readonly kind: "shell" }
  | { readonly kind: "precache"; readonly key: string };

/** Decides how to answer a request. */
export function route(request: RouteRequest, origin: string, manifest: ShellManifest): Route {
  const url = new URL(request.url);
  if (request.method !== "GET" || url.origin !== origin) return { kind: "network" };
  if (NETWORK_ONLY.some((prefix) => url.pathname.startsWith(prefix))) return { kind: "network" };
  if (request.mode === "navigate") return { kind: "shell" };
  const entry = manifest.entries.find((candidate) => candidate.url === url.pathname);
  if (entry === undefined) return { kind: "network" };
  return { kind: "precache", key: entryKey(entry) };
}

/** The shell document's cache key. */
function shellKey(manifest: ShellManifest): string | undefined {
  const index = manifest.entries.find((entry) => entry.url === "/index.html");
  return index === undefined ? undefined : entryKey(index);
}

/**
 * Answers a request from the shell cache, or returns `undefined` to let
 * the browser fetch it normally. A cache miss falls back to the network.
 */
export async function respond(
  request: RouteRequest,
  context: {
    readonly caches: CacheStorage;
    readonly origin: string;
    readonly manifest: ShellManifest;
    readonly fetch: (url: string) => Promise<Response>;
  },
): Promise<Response | undefined> {
  const decision = route(request, context.origin, context.manifest);
  if (decision.kind === "network") return undefined;
  const key = decision.kind === "shell" ? shellKey(context.manifest) : decision.key;
  if (key === undefined) return undefined;
  const cache = await context.caches.open(shellCacheName(context.manifest));
  const hit = await cache.match(key);
  if (hit !== undefined) return hit;
  return context.fetch(decision.kind === "shell" ? request.url : key);
}

/** A notification the worker shows for a push message. */
export interface PushNotice {
  readonly title: string;
  readonly body: string;
  readonly tag: string | undefined;
  /** Same-origin path opened on click. */
  readonly url: string;
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

/**
 * Reads a push payload sent by the notifications service:
 * `{"title", "body", "url", "tag"}`. Anything malformed still shows a
 * generic notice, because browsers penalise pushes that show nothing.
 * Links to another origin are replaced by the inbox.
 */
export function readPush(payload: string | undefined, origin: string): PushNotice {
  let data: Record<string, unknown> = {};
  try {
    const parsed: unknown = JSON.parse(payload ?? "{}");
    if (typeof parsed === "object" && parsed !== null) data = parsed as Record<string, unknown>;
  } catch {
    // Fall through to the generic notice.
  }
  let url = "/inbox";
  const raw = text(data["url"]);
  if (raw !== undefined) {
    const parsed = new URL(raw, origin);
    if (parsed.origin === origin) url = `${parsed.pathname}${parsed.search}${parsed.hash}`;
  }
  return {
    title: text(data["title"]) ?? "Vellum",
    body: text(data["body"]) ?? "You have a new notification.",
    tag: text(data["tag"]),
    url,
  };
}

/** A window the worker controls. */
export interface WindowClientLike {
  readonly url: string;
  focus(): Promise<unknown>;
  navigate(url: string): Promise<unknown>;
}

/** The worker's `clients` object, reduced to what a click needs. */
export interface ClientsLike {
  matchAll(options: {
    type: "window";
    includeUncontrolled: boolean;
  }): Promise<readonly WindowClientLike[]>;
  openWindow(url: string): Promise<unknown>;
}

/** Opens a notification's link: reuse an open window, else open one. */
export async function openNotice(
  clients: ClientsLike,
  origin: string,
  path: string,
): Promise<void> {
  const target = new URL(path, origin).href;
  const windows = await clients.matchAll({ type: "window", includeUncontrolled: true });
  const exact = windows.find((client) => client.url === target);
  if (exact !== undefined) {
    await exact.focus();
    return;
  }
  const [first] = windows;
  if (first !== undefined) {
    await first.navigate(target);
    await first.focus();
    return;
  }
  await clients.openWindow(target);
}
