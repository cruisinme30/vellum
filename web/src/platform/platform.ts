// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import { isStandalone } from "./pwa/install";
import { registerServiceWorker, type WorkerHandle } from "./pwa/register";

/**
 * The seam between the one UI and the shell it runs in (ADR 0023, D97).
 * Screens ask the platform, never `window.vellumDesktop` or browser APIs
 * directly, so the same build runs in a tab, as a PWA and in Electron.
 */

/** Where the app is running. */
type PlatformKind = "browser" | "pwa" | "desktop";

/** A notification to show outside the page. */
interface Notice {
  readonly title: string;
  readonly body: string;
  /** Same-origin path to open on click. */
  readonly url: string;
}

/**
 * What the Electron preload script exposes as `window.vellumDesktop`.
 * The desktop package implements it; keep both sides in step.
 */
export interface DesktopBridge {
  readonly version: string;
  readonly os: "darwin" | "win32" | "linux";
  /** The Vellum server this window talks to. */
  serverUrl(): Promise<string>;
  /** System-browser sign-in (OAuth 2.1 + PKCE, loopback redirect). */
  signIn(): Promise<{ readonly account: string }>;
  signOut(): Promise<void>;
  /** A fresh access token for `Authorization: Bearer`, or undefined when signed out. */
  accessToken(): Promise<string | undefined>;
  openExternal(url: string): Promise<void>;
  notify(notice: Notice): Promise<void>;
  /** Registers a handler for `vellum://` links opened from other apps. */
  onOpenUrl(listener: (path: string) => void): () => void;
}

/** What screens may ask of the shell. */
export interface Platform {
  readonly kind: PlatformKind;
  /** Desktop features: whole-space offline copies and private-space search/MCP. */
  readonly desktop: DesktopBridge | undefined;
  /** Opens a link outside the app (system browser on desktop). */
  openExternal(url: string): void;
  /** Shows a system notification if the person allowed it. */
  notify(notice: Notice): Promise<boolean>;
}

/** Reads the bridge the desktop preload script installed, if any. */
export function desktopBridge(scope: object = globalThis): DesktopBridge | undefined {
  const candidate = (scope as { vellumDesktop?: unknown }).vellumDesktop;
  return typeof candidate === "object" && candidate !== null
    ? (candidate as DesktopBridge)
    : undefined;
}

/** Builds the platform for the current environment. */
export function createPlatform(scope: object = globalThis): Platform {
  const desktop = desktopBridge(scope);
  if (desktop !== undefined) {
    return {
      kind: "desktop",
      desktop,
      openExternal(url) {
        void desktop.openExternal(url);
      },
      async notify(notice) {
        await desktop.notify(notice);
        return true;
      },
    };
  }
  return {
    kind: isStandalone(scope) ? "pwa" : "browser",
    desktop: undefined,
    openExternal(url) {
      globalThis.open(url, "_blank", "noopener,noreferrer");
    },
    async notify(notice) {
      if (!("Notification" in globalThis) || Notification.permission !== "granted") return false;
      const registration = await navigator.serviceWorker.getRegistration();
      if (registration === undefined) return false;
      await registration.showNotification(notice.title, { body: notice.body, data: notice.url });
      return true;
    },
  };
}

/** Options for {@link startPlatform}. */
export interface StartOptions {
  /** Development builds serve no service worker. */
  readonly production: boolean;
  readonly onUpdateReady?: () => void;
}

/**
 * Start-up work for the shell: registers the service worker in the
 * browser and asks the browser not to evict offline data.
 */
export async function startPlatform(
  platform: Platform,
  options: StartOptions,
): Promise<WorkerHandle | undefined> {
  if (platform.kind === "desktop" || !options.production) return undefined;
  const storage = (navigator as Partial<Navigator>).storage;
  // Not awaited: Firefox asks the person first, and the answer can take
  // forever. Offline start-up must not wait on that prompt.
  void storage?.persist().catch(() => false);
  return registerServiceWorker(
    options.onUpdateReady === undefined ? {} : { onUpdateReady: options.onUpdateReady },
  );
}
